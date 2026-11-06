#define UNICODE
#define _UNICODE
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
/* 仅合成身份的SDK ABI验收客户端, 不包含游戏或账号认证. */
static volatile LONG callback_result;
static volatile LONG query_results;
static int antifatigue_request_id;
static const uint32_t app_id=1289;
static const char extra_payload[]="{\"fixture\":\"synthetic\",\"version\":1}";
static void report(const char *stage, unsigned long value) {
    char line[256];
    int length = snprintf(line, sizeof(line), "{\"stage\":\"%s\",\"value\":%lu}\n", stage, value);
    DWORD written;
    if (length > 0 && (size_t)length < sizeof(line)) {
        /* 跨进程printf曾拆开一行, 整行只写一次以保留阶段记录边界. */
        WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), line, (DWORD)length, &written, NULL);
    }
}

static void WINAPI success(const char *uid, const char *token, uint32_t platform_type, uint32_t channel_id) {
    int ok = uid && token && strcmp(uid, "synthetic-master-uid") == 0
             && strcmp(token, "synthetic-master-token") == 0 && platform_type == 4 && channel_id == 9;
    InterlockedExchange(&callback_result, ok ? 1 : -1);
    report("synthetic_callback_fields_match", ok);
}

static void WINAPI failed(uint32_t error, const char *description) {
    (void)description;
    InterlockedExchange(&callback_result, -1);
    report("sdk_failed_callback", error);
}

static void WINAPI canceled(void) {
    InterlockedExchange(&callback_result, -1);
    report("sdk_cancel_callback", 1);
}

static void WINAPI query_complete(int code, const WCHAR *message, const char *body, int request_id) {
    const char *expected = request_id == 101 ? "{\"fixture\":\"synthetic\",\"queryType\":0}"
                                           : "{\"fixture\":\"synthetic\",\"queryType\":1}";
    int ok = (request_id == 101 || request_id == 102) && code == (request_id == 101 ? 0 : 7)
             && message && wcscmp(message, L"synthetic query") == 0 && body && strcmp(body, expected) == 0;
    if (ok) InterlockedOr(&query_results, request_id == 101 ? 1 : 2);
    else InterlockedExchange(&query_results, -1);
    report("synthetic_query_callback_match", ok);
}

static void WINAPI antifatigue_complete(int code, const char *body, int request_id) {
    int ok = code == 7 && request_id == antifatigue_request_id && body
             && strcmp(body, "{\"fixture\":\"synthetic\",\"queryType\":1}") == 0;
    if (ok) InterlockedOr(&query_results, 2);
    else InterlockedExchange(&query_results, -1);
    report("synthetic_antifatigue_callback_match", ok);
}

int main(void) {
    WCHAR exe[MAX_PATH], directory[MAX_PATH];
    if (!GetModuleFileNameW(NULL,exe,MAX_PATH)) return 2;
    wcscpy(directory,exe);
    WCHAR *slash=wcsrchr(directory,L'\\');
    if (!slash) return 2;
    slash[1]=0;
    WNDCLASSW cls={0}; cls.lpfnWndProc=DefWindowProcW; cls.hInstance=GetModuleHandleW(NULL);
    cls.lpszClassName=L"OdditySdkAcceptanceClient";
    if (!RegisterClassW(&cls)) return 2;
    HWND window=CreateWindowW(cls.lpszClassName,L"Oddity SDK acceptance",WS_OVERLAPPEDWINDOW,0,0,100,100,NULL,NULL,cls.hInstance,NULL);
    if (!window) return 2;
    ShowWindow(window,SW_SHOW);
    const int union_client=1;
        WCHAR dll[MAX_PATH];
        swprintf(dll, MAX_PATH, L"%lsWPGameClientSDK_64.dll", directory);
        SetEnvironmentVariableW(L"SDKFILEPATH", dll);
        WCHAR load_path[MAX_PATH];
        wcscpy(load_path, dll);
        if (union_client) {
            WCHAR sdk_base[MAX_PATH];
            swprintf(sdk_base, MAX_PATH, L"%lsWPGameClientSDK", directory);
            SetEnvironmentVariableW(L"ArcGameSDKSharePath", sdk_base);
            swprintf(load_path, MAX_PATH, L"%lsWPOneSDK_64.dll", directory);
        }
        SetEnvironmentVariableW(L"ArcGameAppId", NULL);
        SetEnvironmentVariableW(L"ArcLoadOverlayDll", L"0");
        HMODULE module = LoadLibraryExW(load_path, NULL, LOAD_WITH_ALTERED_SEARCH_PATH);
        if (!module) { report("sdk_load_failed", GetLastError()); return 4; }
        typedef void *(WINAPI *init_fn)(const WCHAR *, const WCHAR *, const WCHAR *, void *, void *, void *);
        typedef uint32_t (WINAPI *login_fn)(void *, void (WINAPI *)(const char *, const char *, uint32_t, uint32_t),
                                           void (WINAPI *)(uint32_t, const char *), void (WINAPI *)(void));
        typedef const char *(WINAPI *string_getter_fn)(void);
        init_fn init = (init_fn)GetProcAddress(module, union_client ? "OneSdk_Union_Init" : "OneSdk_Init");
        login_fn login = (login_fn)GetProcAddress(module, "OneSdk_DoLoginWithFailedCallback");
        string_getter_fn get_channel_uid = (string_getter_fn)GetProcAddress(module, "OneSdk_ChannelUid");
        string_getter_fn get_channel_token = (string_getter_fn)GetProcAddress(module, "OneSdk_ChannelToken");
        string_getter_fn get_channel_info = union_client ? NULL : (string_getter_fn)GetProcAddress(module, "OneSdk_USerChannelInfo");
        string_getter_fn get_user_ip = (string_getter_fn)GetProcAddress(module, "OneSdk_UserIp");
        string_getter_fn get_login_extra = (string_getter_fn)GetProcAddress(module, "OneSdk_GetOneLoginExtraInfo");
        if (!init || !login || !get_channel_uid || !get_channel_token || (!union_client && !get_channel_info)
            || !get_user_ip || !get_login_extra) return 5;
        WCHAR client_app_id[32];
        swprintf(client_app_id, 32, L"%u", app_id);
        void *handle = init(client_app_id, L"OdditySynthetic", NULL, NULL, NULL, NULL);
        report("sdk_init_handle_present", handle != NULL);
        if (!handle) {  return 6; }
        if (union_client) {
            /* 转发层无此getter导出, 只从它实际加载的客户端模块复核同一缓存. */
            get_channel_info = (string_getter_fn)GetProcAddress(GetModuleHandleW(L"WPGameClientSDK_64.dll"), "OneSdk_USerChannelInfo");
            if (!get_channel_info) return 5;
        }
        uint32_t result = login(handle, success, failed, canceled);
        report("sdk_do_login_result", result);
        ULONGLONG deadline = GetTickCount64() + 10000;
        while (!callback_result && GetTickCount64() < deadline) {
            MSG msg;
            while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            MsgWaitForMultipleObjects(0, NULL, FALSE, 25, QS_ALLINPUT);
        }
        int getters_match = 0;
        int extra_getters_match = 0;
        if (callback_result == 1) {
            const char *uid = get_channel_uid();
            const char *token = get_channel_token();
            const char *info = get_channel_info();
            getters_match = uid && token && info && strcmp(uid, "synthetic-channel-uid") == 0
                            && strcmp(token, "synthetic-channel-token") == 0 && strcmp(info, "{}") == 0;
            report("synthetic_channel_getters_match", getters_match);
            const char *ip = get_user_ip();
            const char *extra = get_login_extra();
            const char *expected_extra = app_id == 1289
                ? "{\"fixture\":\"synthetic\",\"userIp\":\"192.0.2.1\",\"version\":1}" : extra_payload;
            extra_getters_match = ip && extra && strcmp(ip, "192.0.2.1") == 0 && strcmp(extra, expected_extra) == 0;
            report("synthetic_extra_getters_match", extra_getters_match);
        }
        int queries_match = app_id != 1289;
        if (callback_result == 1 && app_id == 1289) {
            typedef void (WINAPI *query_callback)(int, const WCHAR *, const char *, int);
            typedef void (WINAPI *roles_fn)(const char *, query_callback, int);
            typedef void (WINAPI *prelogin_fn)(query_callback, int);
            typedef int (WINAPI *wm_prelogin_fn)(void (WINAPI *)(int, const char *, int));
            roles_fn roles = (roles_fn)GetProcAddress(module, "OneSdk_QueryRoleList");
            prelogin_fn prelogin = (prelogin_fn)GetProcAddress(module, "OneSdk_QueryAntifatiguePreLogin");
            wm_prelogin_fn wm_prelogin = (wm_prelogin_fn)GetProcAddress(module, "WMAntifatigueSdk_QueryAntifatiguePreLogin");
            if (!roles || (union_client ? !wm_prelogin : !prelogin)) return 5;
            roles("synthetic-server", query_complete, 101);
            if (union_client) antifatigue_request_id = wm_prelogin(antifatigue_complete);
            else prelogin(query_complete, 102);
            deadline = GetTickCount64() + 10000;
            while (query_results >= 0 && query_results != 3 && GetTickCount64() < deadline) {
                MSG msg;
                while (PeekMessageW(&msg, NULL, 0, 0, PM_REMOVE)) {
                    TranslateMessage(&msg); DispatchMessageW(&msg);
                }
                MsgWaitForMultipleObjects(0, NULL, FALSE, 25, QS_ALLINPUT);
            }
            queries_match = query_results == 3;
            report("synthetic_query_callbacks_match", queries_match);
        }
        int exit_code = callback_result == 1 && getters_match && extra_getters_match && queries_match ? 0 : 7;

        /* 进程退出释放SDK, 避免未闭合的UnInit路径被误当作必要验证条件. */
        ExitProcess(exit_code);
    return 2;
}
