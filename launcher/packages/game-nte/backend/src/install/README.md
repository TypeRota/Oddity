# 安装探测域

定位官方启动器与游戏文件. 设置里的安装目录优先, 其次是 `NTE_ROOT` / `NTE_INSTALL`,
最后扫描默认安装位置与可移动磁盘. 宿主不在自己的代码里保存这些目录名与探测规则.

## 职责

- 官方启动器根目录探测, 识别 `<install>/<launcherDir>/Config/Config.ini` 形态.
- 候选目录: `~/Games/<安装目录名>`, `Program Files (x86)` / `Program Files`,
  `~/Games/异环`, 工作目录的各级祖先, `/run/media`、`/media`、`/mnt` 的两级挂载目录.
- 默认安装位置与安装根目录: 配置了目录时以它为准, 否则取启动器根目录的父目录.
- 游戏可执行文件 (`Client/WindowsNoEditor/HT/Binaries/Win64/HTGame.exe`, 大小写不敏感)
  与官方记录的游戏版本 (`UserData/Config/Config.ini` 的 `[Game] Version_`).
- 区服收敛为 GameID 与启动器目录名 (国服 1289 / NTELauncher, 国际服 3000001 / NTEGlobal).

## 依赖

- `oddity-runtime`: `InstallLocator` 与 `InstallQuery`; 宿主经
  `GamePackage::install_locator` 取用本域.
- 不依赖宿主 crate; 环境变量、家目录与挂载点都在本域内读取.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte install
```

用临时目录构造安装树. 探测用的家目录、工作目录、环境变量与挂载点由 `Probe`
注入, 只有读写真实进程环境变量的用例需要独占环境锁.

## 限制

- 挂载点扫描固定探测 `/run/media`、`/media`、`/mnt` 的两级目录.
- 版本只读官方 `[Game] Version_`, 不解析启动器版本.
