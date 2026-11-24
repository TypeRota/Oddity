# 官方背景域

读取官方启动器自带的动态背景: 优先 `ResFilesM/<GameID>/bgimgs` 下的视频, 找不到时
回退到同目录的静态壁纸. 主张材只从官方安装目录读取, 不随 Oddity 分发; 开启归档后
按内容哈希把换下来的素材复制到用户数据目录, 供用户切换.

## 职责

- 解析 `bgimgs/config.json` 的 `video` / `imgs` / `first` / `noVideoBg`, 并按官方命名回退.
- 通过命令组声明并处理 `launcher_background` / `launcher_background_archive` /
  `launcher_background_select` / `launcher_background_configure`.
- 归档按文件名与内容哈希标识一套素材; 官方启动器原地覆盖同名文件, 因此不能按
  文件名判断新旧. 保留套数 (2..10, 默认 3) 内的条目留在
  `<数据目录>/backgrounds/nte/<channel>/<id>/`, 索引与归档偏好保存在同级的
  `<channel>.json`.
- `launcher_background` 只返回一对可播放路径: 选中了历史素材时返回那一套,
  否则返回官方当前素材. 目录迁移失败时继续使用上一个可用目录.
- 只接受背景目录下的单层文件名, 配置无法把路径引到目录外.
- 区服在包内收敛为 GameID 与启动器目录名 (国服 1289, 国际服 3000001).

## 依赖

- `oddity-runtime`: 命令组与宿主能力. 官方启动器根目录由
  `HostContext::official_launcher_root` 提供, 该路径来自本包的安装域;
  数据目录由 `HostContext::data_dir` 提供.
- `serde` / `serde_json`: 配置与返回结构.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte background
```

扫描、解析、归档与迁移都用临时目录构造, 命令路径用最小宿主验证, 测试完全离线.

## 限制

- 只读官方配置里的 `[General] GameID`; 其余字段与本域无关.
- 安装位置探测 (设置里的安装目录, 默认位置, 可移动磁盘) 在本包的安装域;
  本域只读 `HostContext::official_launcher_root` 给出的最终路径.
- 归档目录整体不可达时保留索引, 等目录恢复后继续使用.
- 索引锁只覆盖同一进程; 界面与命令行同时运行时不保证并发安全.
