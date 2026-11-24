// 会话服务客户端: 界面只是连上本机套接字的一个客户端.
//
// 服务由第一次连接时拉起, 之后与界面进程相互独立; 界面退出不会结束服务,
// 重新打开时接回同一份权威状态. 服务自身退出或重启时, 界面按需重新拉起它.
import { spawn } from "node:child_process";
import { closeSync, openSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { connect, type Socket } from "node:net";
import { EventEmitter } from "node:events";

interface Reply {
  id?: number;
  result?: unknown;
  error?: unknown;
  event?: string;
  payload?: unknown;
}

/** 拉起服务后等待套接字就绪的最长时间. */
const SPAWN_TIMEOUT = 10000;
/** 单次连接尝试的间隔. */
const RETRY_INTERVAL = 25;

export interface BackendOptions {
  executable: string;
  /** 数据目录; 服务把真实套接字路径写在其中的入口文件里. */
  dataDir: string;
  /** 服务自身的标准错误落盘位置. */
  log: string;
  compatible: boolean;
}

export class Backend extends EventEmitter {
  private readonly options: BackendOptions;
  private socket?: Socket;
  private next = 0;
  private pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: unknown) => void }
  >();
  private connected = false;
  private closed = false;
  private incoming = "";
  /** 拉起服务的子进程; 正常情况由服务自己常驻, 这里只负责首次启动. */
  private launcher?: ReturnType<typeof spawn>;

  constructor(options: BackendOptions) {
    super();
    this.options = options;
  }

  /** 连接服务; 服务未运行时就拉起它. */
  async start(): Promise<void> {
    const socket = await this.reach();
    this.attach(socket);
  }

  private async reach(): Promise<Socket> {
    const deadline = Date.now() + SPAWN_TIMEOUT;
    let spawned = false;
    for (;;) {
      // 真实套接字路径由服务写进入口文件, 客户端不自行拼路径.
      const socket = await tryConnect(
        await readSocketPath(this.options.dataDir),
      );
      if (socket) return socket;
      // 服务没在跑: 冷启动时由界面拉起; 已退出且无人负责时同样补一次.
      if (!spawned) {
        spawned = true;
        this.spawnService();
      }
      if (Date.now() >= deadline)
        throw new Error("无法连接会话服务, 请查看启动日志.");
      await new Promise((resolve) => setTimeout(resolve, RETRY_INTERVAL));
    }
  }

  private spawnService() {
    // 服务自己持有单实例锁, 并发拉起时只有第一个能启动.
    // 标准错误写入日志文件: 管道会随界面退出断开, 让常驻服务失去输出.
    const stderr = openSync(this.options.log, "a", 0o600);
    const child = spawn(
      this.options.executable,
      ["--ipc", ...(this.options.compatible ? ["--recovery-compatible"] : [])],
      { stdio: ["ignore", "ignore", stderr], detached: true },
    );
    closeSync(stderr);
    child.on("error", (error) => process.stderr.write(String(error)));
    // 服务独立常驻, 不随界面退出; 这里只解除引用, 不等待它.
    child.unref();
    this.launcher = child;
    child.on("exit", () => {
      if (this.launcher === child) this.launcher = undefined;
    });
  }

  private attach(socket: Socket) {
    this.socket = socket;
    this.connected = true;
    this.incoming = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk: string) => this.receive(chunk));
    socket.on("error", (error) => this.fail(error));
    socket.on("close", () => void this.lost());
  }

  /// 连接意外断开: 先尝试接回, 接不上再拉起服务.
  private async lost() {
    if (this.closed) return;
    this.fail(new Error("会话服务连接已断开"));
    try {
      const socket = await this.reach();
      this.attach(socket);
      this.emit("reconnected");
    } catch (error) {
      console.error(error);
      this.emit("exit", null);
    }
  }

  private receive(chunk: string) {
    this.incoming += chunk;
    for (;;) {
      const newline = this.incoming.indexOf("\n");
      if (newline < 0) return;
      const line = this.incoming.slice(0, newline);
      this.incoming = this.incoming.slice(newline + 1);
      if (!line) continue;
      let reply: Reply;
      try {
        reply = JSON.parse(line) as Reply;
      } catch (error) {
        this.fail(error);
        return;
      }
      if (reply.event) {
        this.emit("event", reply.event, reply.payload);
        continue;
      }
      const request =
        reply.id === undefined ? undefined : this.pending.get(reply.id);
      if (!request) continue;
      this.pending.delete(reply.id!);
      if (reply.error !== undefined) request.reject(reply.error);
      else request.resolve(reply.result);
    }
  }

  call(command: string, args?: unknown, game?: string): Promise<unknown> {
    if (!this.connected || !this.socket)
      return Promise.reject(new Error("会话服务连接已断开"));
    const id = ++this.next;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket!.write(
        JSON.stringify({
          id,
          command,
          args: args ?? {},
          ...(game ? { game } : {}),
        }) + "\n",
      );
    });
  }

  /** 断开界面与服务的连接; 服务与运行中的会话继续存在. */
  close() {
    if (this.closed) return;
    this.closed = true;
    this.connected = false;
    this.socket?.end();
    this.socket = undefined;
    this.fail(new Error("会话服务连接已关闭"));
  }

  private fail(error: unknown) {
    this.connected = false;
    for (const request of this.pending.values()) request.reject(error);
    this.pending.clear();
  }
}

/** 读取入口文件里的真实套接字路径; 服务尚未启动时返回 null. */
async function readSocketPath(dataDir: string): Promise<string | null> {
  try {
    const text = await readFile(join(dataDir, "ipc-path"), "utf8");
    const path = text.trim();
    return path ? path : null;
  } catch {
    return null;
  }
}

/** 尝试连接一次; 连不上返回 null. */
function tryConnect(path: string | null): Promise<Socket | null> {
  if (!path) return Promise.resolve(null);
  return new Promise((resolvePromise) => {
    const socket = connect(path);
    const settle = (value: Socket | null) => {
      socket.removeAllListeners();
      socket.on("error", () => {});
      resolvePromise(value);
    };
    socket.once("connect", () => settle(socket));
    socket.once("error", () => {
      socket.destroy();
      settle(null);
    });
  });
}
