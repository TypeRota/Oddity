# 异环游戏包

真实游戏包. 后端按领域承接命令组, 前端提供页面组件; 两者都通过契约与宿主交互.

## 结构

```
game-nte/
├── backend/             Rust 包, 当前包含公告, 官方背景, 塔吉多签到, 游戏账号与更新域
│   └── src/
│       ├── lib.rs       包身份与领域命令组
│       ├── bulletin/    公告抓取, 解析, 命令分发与测试
│       ├── background/  官方背景扫描, 解析, 命令分发与测试
│       ├── attendance/  塔吉多签到, 账号与游戏状态, 命令分发与测试
│       ├── auth/        国服游戏账号, 登录, 会话与密钥环存储
│       ├── install/     官方启动器与游戏文件探测
│       └── update/      更新域: 区服与路径常量, 游戏资源引擎, 启动器自更新
└── ui/                  Vue 包
    ├── package.json
    └── src/
        ├── index.ts          包入口
        ├── pages.ts          页面声明与可见条件 (宿主只取这份元数据)
        ├── contribution.ts   页面组件、首页卡片与设置分区的清单
        ├── game.ts           游戏展示名、安装目录提示与海报文件名
        ├── preview.ts        公告与签到命令的浏览器演示
        ├── domains/bulletin/     公告页与相关组件
        ├── domains/player-info/  首页玩家信息卡
        └── domains/settings/     设置分区与签到界面
```

## 后端

- 包 id 为 `nte`, 显示名为 "异环".
- 公告域声明 `fetch_bulletins`, 参数只有区服 (`cn` / `global`);
  每个来源独立失败, 返回 `BulletinFeed.errors`.
- 背景域声明 `launcher_background`, 无参数; 官方启动器根目录由宿主提供.
- 签到域声明 `attendance_*`; 凭据进平台密钥环, 区服来自宿主只读设置快照.
- 账号域声明 `game_accounts` / `game_send_code` / `game_login` /
  `game_account_remove`; 认证配置来自官方启动器, 账号进平台密钥环, 国服能力门
  同时供宿主直接启动使用.
- 更新域声明 `check_update` 与 7 条 `game_download_*` 下载命令, 并承接游戏资源
  更新引擎 (清单/差分/下载/账本/预下载/可选资源) 与自动下载调度; 启动器自更新
  也在此. 区服常量放在 `update/region.rs`, 安装与区服信息经 `InstallLocator` 提供给宿主.
- 依赖 `oddity-runtime` 与 `oddity-platform`; 不依赖宿主实现.

## 前端

页面、首页卡片与设置分区都由包声明, 宿主只负责壳与导航:

- `pages.ts` 声明公告页的位置与可见条件, `contribution.ts` 把 `BulletinPage`
  绑到这份声明上并汇总包内全部 UI 贡献; 宿主经生成物装配, 不再直接引用包.
- `game.ts` 声明游戏展示名: 国服为 "异环", 国际服为 "NTE". 宿主首页标题取
  这份声明, 不再按发行渠道拼文案.
- `contribution.ts` 声明 "游戏与启动" 与 "账号与签到" 两个设置分区, 连同
  检索项、顺序与可见条件; 宿主设置页按贡献表装配.
- `domains/player-info` 的 `PlayerInfoCard` 由 `contribution.ts` 挂到首页.
- `game.ts` 同时给出安装目录提示与游玩海报文件名, 更新页与海报组件只读这份
  声明; 没有声明时宿主用通用文案.
- `preview.ts` 用 `PreviewContribution` 声明公告与塔吉多签到命令的浏览器演示;
  生成物把它汇总进宿主预览表, 模拟数据不再留在宿主.
- 修改 `pages.ts`, `contribution.ts`, `game.ts` 或 `preview.ts` 后运行
  `cargo run -p ui-registry-gen`, 并提交 `launcher/ui/src/host/generated/` 下的
  两个生成物; CI 用 `--check` 校验漂移.

这些声明都以 `activeGame === "nte"` 为可见条件: 切换到其他游戏时页面、卡片与
设置分区一起卸载. 包 UI 只使用 `@host/api`, `@host/ui-kit`, `@packages` 与
宿主依赖树, 不引用宿主内部实现.

## 测试

```bash
cd launcher && cargo test -p oddity-game-nte
cd launcher/ui && NODE_ENV=test pnpm test -- tests/unit/bulletin-page.test.ts
```

网络路径用回环 HTTP 服务替代外网, 解析规则用固定页面片段验证, 测试完全离线.
