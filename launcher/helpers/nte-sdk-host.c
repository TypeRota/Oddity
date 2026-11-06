#define UNICODE
#define _UNICODE
#include <winsock2.h>
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

/* 仅提供游戏SDK宿主, 游戏由Rust服务启动和管理. */
#define APP_ID 1289u
#define HOST_NOTIFY (WM_APP + 0x130)
#define CONTROL_CLOSED (WM_APP + 0x133)
#define MAGIC 0x6b647331u

static HANDLE pipe_handle = INVALID_HANDLE_VALUE;
static HWND client_window;
static DWORD host_pid;
static char env_reply[4096];
static WCHAR fixture[3][MAX_PATH];
static CRITICAL_SECTION write_lock;
static int host_result = 1;
static uint32_t app_id = APP_ID;
static DWORD connected_client_pid;
static HWND server_window;
static unsigned char session_input[65536 - 20];
static unsigned char *session_login, *session_ip, *session_extra;
static DWORD session_extra_size;
static int session_sent;
static int session_delivery_ok;
static SOCKET control_socket = INVALID_SOCKET;
static int winsock_started;
static int send_frame(unsigned char *buffer, DWORD size, int encrypt);

static void report(const char *stage, unsigned long value) {
    char line[256];
    int length = snprintf(line, sizeof(line), "{\"stage\":\"%s\",\"value\":%lu}\n", stage, value);
    DWORD written;
    if (length > 0 && (size_t)length < sizeof(line)) {
        /* 跨进程printf曾拆开一行, 整行只写一次以保留阶段记录边界. */
        WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), line, (DWORD)length, &written, NULL);
    }
}

static void put32(unsigned char *buffer, size_t offset, uint32_t value) {
    memcpy(buffer + offset, &value, sizeof(value));
}

static void close_control(void) {
    if (control_socket != INVALID_SOCKET) {
        shutdown(control_socket, SD_BOTH);
        closesocket(control_socket);
        control_socket = INVALID_SOCKET;
    }
    if (winsock_started) {
        WSACleanup();
        winsock_started = 0;
    }
    SecureZeroMemory(session_input, sizeof(session_input));
}

static int read_exact(void *buffer, DWORD size) {
    DWORD total = 0;
    while (total < size) {
        int received = recv(control_socket, (char *)buffer + total, (int)(size - total), 0);
        if (received <= 0) return 0;
        total += (DWORD)received;
    }
    return 1;
}

static DWORD WINAPI control_lifetime(void *unused) {
    (void)unused;
    /* 初始会话后, 同一连接接收Rust完成业务查询后的cmd49. */
    for (;;) {
        uint32_t size;
        int count = recv(control_socket, (char *)&size, sizeof(size), MSG_WAITALL);
        if (count == 0) { PostMessageW(server_window, CONTROL_CLOSED, 0, 0); break; }
        unsigned char frame[65536];
        if (count != sizeof(size) || size < 0x422 || size > sizeof(frame) || !read_exact(frame, size)) {
            report("invalid_query_response_size", 1);
            PostMessageW(server_window, CONTROL_CLOSED, 2, 0); break;
        }
        uint32_t header[4];
        memcpy(header, frame, sizeof(header));
        if (header[0] != MAGIC || header[1] != 49 || header[2] != size || header[3] != size - 17
            || !wmemchr((const WCHAR *)(frame + 0x1c), 0, 512) || frame[size - 6] != 0) {
            report("invalid_query_response_frame", 1);
            PostMessageW(server_window, CONTROL_CLOSED, 2, 0); break;
        }
        report("query_response_sent", send_frame(frame, size, 1));
    }
    return 0;
}

struct window_search {
    DWORD pid;
    HWND result;
    unsigned count;
};

static BOOL CALLBACK find_client_window(HWND window, LPARAM context) {
    struct window_search *search = (struct window_search *)context;
    DWORD pid = 0;
    GetWindowThreadProcessId(window, &pid);
    if (pid != search->pid || !IsWindowVisible(window) || GetAncestor(window, GA_ROOT) != window
        || GetWindow(window, GW_OWNER)) return TRUE;
    WCHAR class_name[128];
    if (!GetClassNameW(window, class_name, 128)
        || wcscmp(class_name, L"CPGPMObilePcGameBridgeWnd") == 0) return TRUE;
    search->result = window;
    ++search->count;
    return TRUE;
}

static int send_frame(unsigned char *buffer, DWORD size, int encrypt) {
    DWORD written = 0;
    if (encrypt) {
        for (DWORD i = 12; i < size; ++i) buffer[i] ^= (unsigned char)host_pid;
    }
    EnterCriticalSection(&write_lock);
    BOOL ok = WriteFile(pipe_handle, buffer, size, &written, NULL);
    DWORD error = ok ? 0 : GetLastError();
    LeaveCriticalSection(&write_lock);
    if (!ok || written != size) {
        report("pipe_write_failed", error);
        return 0;
    }
    return 1;
}

static DWORD WINAPI pipe_worker(void *unused) {
    (void)unused;
    if (!ConnectNamedPipe(pipe_handle, NULL) && GetLastError() != ERROR_PIPE_CONNECTED) {
        report("pipe_connect_failed", GetLastError());
        return 1;
    }
    unsigned char incoming[65536];
    DWORD received;
    while (ReadFile(pipe_handle, incoming, sizeof(incoming), &received, NULL)) {
        if (received < 12) {
            report("short_client_frame", received);
            break;
        }
        uint32_t header[3];
        memcpy(header, incoming, sizeof(header));
        if (header[0] != MAGIC || header[2] != received) {
            report("invalid_client_frame", received);
            break;
        }
        report("client_command", header[1]);
        if (header[1] == 40) {
            if (received != 0x214 || !connected_client_pid) {
                report("invalid_query_request_size", received); continue;
            }
            for (DWORD i = 12; i < received; ++i) incoming[i] ^= (unsigned char)connected_client_pid;
            if (!memchr(incoming + 0x10, 0, 512)) {
                report("invalid_query_request_string", 1); continue;
            }
            uint32_t query_type, request_id;
            memcpy(&query_type, incoming + 12, 4);
            memcpy(&request_id, incoming + 0x210, 4);
            report("query_type", query_type);
            report("query_request_id", request_id);
            unsigned char forwarded[0x218];
            memcpy(forwarded, &received, 4);
            memcpy(forwarded + 4, incoming, received);
            DWORD sent = 0;
            while (sent < sizeof(forwarded)) {
                int count = send(control_socket, (const char *)forwarded + sent, sizeof(forwarded) - sent, 0);
                if (count <= 0) break;
                sent += (DWORD)count;
            }
            if (sent != sizeof(forwarded)) {
                report("query_forward_failed", 1);
                PostMessageW(server_window, CONTROL_CLOSED, 2, 0); break;
            }
            continue;
        }
        if (header[1] != 1000) continue;
        if (received != 0x31c || incoming[0x10] != 1) {
            report("unexpected_start_request", received);
            break;
        }
        memcpy(&connected_client_pid, incoming + 12, sizeof(connected_client_pid));
        if (!connected_client_pid) {
            report("invalid_client_pid", 0);
            break;
        }
        report("connected_client_pid", connected_client_pid);
        unsigned char reply[0x1624] = {0};
        put32(reply, 0, MAGIC);
        put32(reply, 4, 1001);
        put32(reply, 8, sizeof(reply));
        memcpy(reply + 12, env_reply, strlen(env_reply) + 1);
        memcpy(reply + 0x100c, fixture[0], sizeof(fixture[0]));
        memcpy(reply + 0x1214, fixture[1], sizeof(fixture[1]));
        memcpy(reply + 0x141c, fixture[2], sizeof(fixture[2]));
        if (!send_frame(reply, sizeof(reply), 0)) break;
        report("bootstrap_environment_sent", 1);
    }
    return 0;
}

static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == HOST_NOTIFY) {
        if (wparam != 1 || session_sent) return 0;
        struct window_search search = {connected_client_pid, NULL, 0};
        EnumWindows(find_client_window, (LPARAM)&search);
        if (!connected_client_pid || search.count != 1) {
            report("game_window_candidate_count", search.count);
            host_result = 11; PostQuitMessage(host_result); return 0;
        }
        client_window = search.result;
        session_sent = 1;
        unsigned char window_frame[20] = {0};
        put32(window_frame, 0, MAGIC); put32(window_frame, 4, 9); put32(window_frame, 8, sizeof(window_frame));
        uint64_t game_window = (uint64_t)(uintptr_t)client_window;
        memcpy(window_frame + 12, &game_window, sizeof(game_window));
        session_delivery_ok = send_frame(window_frame, sizeof(window_frame), 0)
            && send_frame(session_ip, 0x70, 1) && send_frame(session_extra, session_extra_size, 1)
            && send_frame(session_login, 0x594, 1);
        report("session_frames_sent", session_delivery_ok);
        if (!session_delivery_ok) { host_result = 13; PostQuitMessage(host_result); }
        return 0;
    }
    if (message == CONTROL_CLOSED) {
        host_result = wparam == 2 ? 16 : session_delivery_ok ? 0 : 12;
        PostQuitMessage(host_result); return 0;
    }
    if (message == WM_CLOSE) return 0;
    return DefWindowProcW(window, message, wparam, lparam);
}

int WINAPI WinMain(HINSTANCE instance, HINSTANCE previous, LPSTR command, int show) {
    (void)previous; (void)show;
    const char *port_text = command;
    while (*port_text == ' ') ++port_text;
    WCHAR exe[MAX_PATH];
    DWORD path_size = GetModuleFileNameW(NULL, exe, MAX_PATH);
    if (!path_size || path_size >= MAX_PATH) return 2;
    WNDCLASSW cls = {0}; cls.lpfnWndProc = window_proc; cls.hInstance = instance;
    cls.lpszClassName = L"OddityGameSdkHost";
    if (!RegisterClassW(&cls)) return 2;
    const WCHAR *title = L"Oddity Game SDK Host";
    HWND window = CreateWindowW(cls.lpszClassName, title, WS_OVERLAPPEDWINDOW,
                               0, 0, 100, 100, NULL, NULL, instance, NULL);
    if (!window) return 2;
    server_window = window;
    unsigned port = 0;
    for (const char *digit = port_text; *digit; ++digit) {
        if (*digit < '0' || *digit > '9' || port > 6553) return 2;
        port = port * 10 + (unsigned)(*digit - '0');
    }
    if (!port || port > 65535) return 2;
    char nonce[65] = {0};
    DWORD nonce_size = GetEnvironmentVariableA("ODDITY_HANDOFF_NONCE", nonce, sizeof(nonce));
    SetEnvironmentVariableA("ODDITY_HANDOFF_NONCE", NULL);
    int nonce_valid = nonce_size == 64;
    for (unsigned i = 0; nonce_valid && i < 64; ++i) {
        nonce_valid = (nonce[i] >= '0' && nonce[i] <= '9') || (nonce[i] >= 'a' && nonce[i] <= 'f');
    }
    if (!nonce_valid) {
        SecureZeroMemory(nonce, sizeof(nonce));
        report("invalid_control_nonce", 1); return 16;
    }
    WSADATA data;
    if (WSAStartup(MAKEWORD(2, 2), &data) != 0) {
        SecureZeroMemory(nonce, sizeof(nonce));
        report("control_init_failed", 1); return 16;
    }
    winsock_started = 1;
    control_socket = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    struct sockaddr_in address = {0};
    address.sin_family = AF_INET;
    address.sin_port = htons((u_short)port);
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    if (control_socket == INVALID_SOCKET
        || connect(control_socket, (struct sockaddr *)&address, sizeof(address)) == SOCKET_ERROR) {
        SecureZeroMemory(nonce, sizeof(nonce));
        close_control(); report("control_connect_failed", 1); return 16;
    }
    unsigned sent = 0;
    while (sent < 64) {
        int count = send(control_socket, nonce + sent, (int)(64 - sent), 0);
        if (count <= 0) break;
        sent += (unsigned)count;
    }
    SecureZeroMemory(nonce, sizeof(nonce));
    if (sent != 64) {
        close_control(); report("control_nonce_send_failed", 1); return 16;
    }
    /* 入口一次验证会话包, 后续消息处理只使用确定的帧边界. */
    report("serve_input_begin", 1);
    uint32_t header[5];
    if (!read_exact(header, sizeof(header))
        || header[0] != 0x5253444fu || header[1] != 1 || header[2] != 0x594 || header[3] != 0x70
        || header[4] < 17 || header[4] > sizeof(session_input) - 0x594 - 0x70) {
        close_control(); report("invalid_session_header", 1); return 15;
    }
    DWORD payload_size = header[2] + header[3] + header[4];
    if (!read_exact(session_input, payload_size)) {
        close_control(); report("truncated_session_input", 1); return 15;
    }
    session_login = session_input;
    session_ip = session_login + 0x594;
    session_extra = session_ip + 0x70;
    session_extra_size = header[4];
    uint32_t login_header[3], ip_header[3], extra_header[4];
    memcpy(login_header, session_login, sizeof(login_header));
    memcpy(ip_header, session_ip, sizeof(ip_header));
    memcpy(extra_header, session_extra, sizeof(extra_header));
    if (login_header[0] != MAGIC || login_header[1] != 2 || login_header[2] != 0x594
        || ip_header[0] != MAGIC || ip_header[1] != 0x3ed || ip_header[2] != 0x70
        || extra_header[0] != MAGIC || extra_header[1] != 0x40d || extra_header[2] != session_extra_size
        || extra_header[3] != session_extra_size - 17 || session_extra[session_extra_size - 1] != 0
        || !memchr(session_login + 12, 0, 64) || !memchr(session_login + 0x4c, 0, 128)
        || !memchr(session_login + 0xd4, 0, 64) || !memchr(session_login + 0x114, 0, 128)
        || !memchr(session_login + 0x194, 0, 1024) || !memchr(session_ip + 12, 0, 100)) {
        close_control(); report("invalid_session_frames", 1); return 15;
    }
    report("serve_input_accepted", 1);
    host_pid = GetCurrentProcessId();
    InitializeCriticalSection(&write_lock);
    WCHAR pipe_name[100], mapping_name[100], mutex_name[100];
    swprintf(pipe_name, 100, L"OdditySdkPipe_%lu", host_pid);
    swprintf(mapping_name, 100, L"Local\\OdditySdkMapping_%lu", host_pid);
    swprintf(mutex_name, 100, L"Local\\OdditySdkMutex_%lu", host_pid);
    SetEnvironmentVariableW(L"ArcGameP", pipe_name);
    SetEnvironmentVariableW(L"ArcGameAppId", NULL);
    SetEnvironmentVariableW(L"gameRealOemId", NULL);
    SetEnvironmentVariableW(L"dxtools", NULL);
    SetEnvironmentVariableW(L"ArcLoadOverlayDll", L"0");
    WCHAR full_pipe[160];
    swprintf(full_pipe, 160, L"\\\\.\\PIPE\\%ls", pipe_name);
    pipe_handle = CreateNamedPipeW(full_pipe, PIPE_ACCESS_DUPLEX,
                                  PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                                  1, 65536, 65536, 1000, NULL);
    WCHAR event_name[100];
    swprintf(event_name, 100, L"Global\\CoreClient_StartEvent_%u", app_id);
    HANDLE event = CreateEventW(NULL, TRUE, TRUE, event_name);
    HANDLE mapping = CreateFileMappingW(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE, 0, 0x1000, mapping_name);
    HANDLE mutex = CreateMutexW(NULL, FALSE, mutex_name);
    if (pipe_handle == INVALID_HANDLE_VALUE || !event || !mapping || !mutex) { close_control(); return 8; }
    unsigned char *memory = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0x1000);
    if (!memory) { close_control(); return 8; }
    memset(memory, 0, 0x1000);
    put32(memory, 0, 1);
    put32(memory, 8, app_id);
    for (int i = 0; i < 3; ++i) wcscpy(fixture[i], exe);
    uint32_t title_xor = 0;
    for (const WCHAR *p = title; *p; ++p) title_xor ^= (uint16_t)*p;
    int32_t anchor_w = (int32_t)((uint32_t)(uintptr_t)window ^ app_id);
    int32_t anchor_v = (int32_t)(1u - (title_xor ^ app_id));
    snprintf(env_reply, sizeof(env_reply),
             "ArcGameAppId|=|%u|;|CoreGameId|=|%u|;|ArcMoblieNotify|=|%u|;|"
             "ArcProcessId|=|%lu|;|anchor_w|=|%d|;|anchor_v|=|%d|;|StandardPlatform|=|1|;|"
             "wmgplaunch|=|2|;|ArcDataIndex|=|%u|;|ArcMId|=|Local\\OdditySdkMapping_%lu|;|"
             "ArcMMId|=|Local\\OdditySdkMutex_%lu|;|ArcLoadOverlayDll|=|0",
             app_id, app_id, HOST_NOTIFY, host_pid, anchor_w, anchor_v, app_id, host_pid, host_pid);
    HANDLE worker = CreateThread(NULL, 0, pipe_worker, NULL, 0, NULL);
    HANDLE control_thread = CreateThread(NULL, 0, control_lifetime, NULL, 0, NULL);
    if (!worker || !control_thread) { close_control(); return 9; }
    if (send(control_socket, (const char *)&host_pid, sizeof(host_pid), 0) != sizeof(host_pid)) {
        close_control(); return 16;
    }
    report("sdk_host_ready", host_pid);
    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0) > 0) { TranslateMessage(&msg); DispatchMessageW(&msg); }
    close_control();
    CloseHandle(control_thread); CloseHandle(worker);
    UnmapViewOfFile(memory); CloseHandle(mapping); CloseHandle(mutex); CloseHandle(event); CloseHandle(pipe_handle);
    DestroyWindow(window);
    return host_result;
}
