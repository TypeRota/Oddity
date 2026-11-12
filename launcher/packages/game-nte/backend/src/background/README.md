# 官方背景域

读取官方启动器自带的动态背景: 优先 `ResFilesM/<GameID>/bgimgs` 下的视频, 找不到时
回退到同目录的静态壁纸. 素材只在本地读取, 不复制也不随 Oddity 分发.

## 职责

- 解析 `bgimgs/config.json` 的 `video` / `imgs` / `first` / `noVideoBg`, 并按官方命名回退.
- 通过命令组声明并处理 `launcher_background`, 返回 `LauncherBackgroundMedia`.
- 只接受背景目录下的单层文件名, 配置无法把路径引到目录外.
- 区服在包内收敛为 GameID 与启动器目录名 (国服 1289, 国际服 3000001).

## 依赖

- `oddity-runtime`: 命令组与宿主能力. 官方启动器根目录由
  `HostContext::official_launcher_root` 提供, 该路径来自本包的安装域.
- `serde` / `serde_json`: 配置与返回结构.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte background
```

扫描与解析用临时目录构造安装树, 命令路径用最小宿主验证, 测试完全离线.

## 限制

- 只读官方配置里的 `[General] GameID`; 其余字段与本域无关.
- 安装位置探测 (设置里的安装目录, 默认位置, 可移动磁盘) 在本包的安装域;
  本域只读 `HostContext::official_launcher_root` 给出的最终路径.
