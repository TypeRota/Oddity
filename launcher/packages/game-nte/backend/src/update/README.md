# 更新域

《异环》的两条更新链路都归这里: 官方启动器自更新 (Version.ini/AllFiles.xml)
与游戏资源更新 (PatcherSDK 的 ResList/lastdiff/PatcherXML0), 以及 `check_update`
命令.

## 职责

- 启动器自更新: 读取官方启动器配置, 校验 ZIP 文件, 规划安装; 命令组声明
  `check_update`, 返回载荷与搬入前的宿主实现逐字段一致.
- 游戏资源更新: 拉取 `config.xml` 与加密的 `ResList.bin.zip`, 建立补丁链与整包
  的成本模型, 校验本地文件并按 HDIFF13 差分或整包落盘, 维护 PatcherSDK 账本与
  可选资源 (pakchunk) 选择记录, 管理跨版本预下载缓存.
- 补丁应用直接使用 `oddity_platform::diff`, 路径安全原语来自
  `oddity_platform::paths`, 更新协议自己的单段路径校验在 `paths.rs`.
- `region.rs` 持有区服与安装布局常量; 经 `InstallLocator` 提供给宿主,
  宿主不保留副本.

## 模块

- `check.rs`: 远端清单与本地状态对账, 生成更新计划与可选资源状态.
- `config.rs`: 由官方 `PatcherConfig.json`/`PatcherPath.json`/`PatcherCheckConfig.json`
  组装 `GameUpdateConfig`.
- `download.rs`: 并发下载, 断点续传, 差分/整包应用, 磁盘预算与进度回调.
- `ledger.rs`: 回写 PatcherSDK 的 `ResList.xml` 账本.
- `manifest.rs`: ResList/lastdiff/RemoteConfig 解析与模型.
- `optional.rs`: pakchunk 可选资源目录与选择记录.
- `plan.rs`: 补丁链与整包的时间成本模型与吞吐校准.
- `preload.rs`: 预下载缓存目录, 完成记录与清理.
- `pxml.rs`: PatcherXML0 的 AES-128-CBC 加解密与压缩.
- `launcher/`: 官方启动器自更新的配置, 清单, 版本, 文件与安装规划.
- `region.rs` / `paths.rs`: 区服与布局常量, 更新协议路径原语.
- `state.rs` / `commands.rs`: 下载状态 (包级 static) 与 7 条下载命令.
- `auto.rs`: 自动下载调度 (时段/计费网络/取消冷却/失败退避), 宿主启动时以
  `auto::start(host, stop)` 启动, 关闭时先停调度再取消任务.
- `testing.rs` (单元测试): 假宿主, 事件订阅与状态注入.

## 依赖

- `oddity-platform`: 共享 HTTP 连接池, 哈希, 差分, 限速, 磁盘查询, 带取消检查的
  流复制, 路径安全与临时文件.
- `oddity-runtime`: 命令组与宿主能力 (`HostContext::settings`, `activity`,
  `official_launcher_root`).
- `serde` / `serde_json` / `quick-xml` / `zip` / `flate2` / `aes` / `cbc` /
  `cipher` / `md-5` / `ureq` / `tokio`.
- 不依赖宿主实现, 只经 `oddity-runtime` 的能力缝.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte update
cd launcher && cargo test -p oddity-game-nte --test virtual_cdn --test cdn_probe
```

网络路径用回环 HTTP 服务替代外网; 清单, 加密 ResList 与 ZIP 在测试里现场构造.
虚拟 CDN 与 CDN 探测集成测试在包的 `backend/tests/`, HDIFF13 夹具在
`backend/tests/fixtures/`. `check_update` 用假宿主快照与回环服务走完整命令入口并
逐字段断言返回 JSON.

## 限制

- 调度只在启动器运行期间工作; 关闭后的无人值守下载不在本域范围.
- UI 的 `CheckResult` / `DownloadStatus` 类型仍是宿主手抄镜像.
- 只覆盖《异环》; 其他游戏没有更新布局.
