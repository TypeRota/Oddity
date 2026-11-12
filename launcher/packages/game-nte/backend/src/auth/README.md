# 游戏账号域

国服游戏账号: 短信验证码, 密码与短信登录, 主账号交换, SDK 查询与密钥环存储.
命令组声明并处理 `game_accounts` / `game_send_code` / `game_login` /
`game_account_remove`; 参数, 返回与错误载荷与宿主实现逐字段一致.

## 职责

- 维护老虎账号协议: 登录, 短信验证码, 主账号交换与游戏侧 SDK 查询.
- 从官方启动器的 `ResData/1289.dat` 读取认证配置: `game.ini` 里的应用密钥
  与加密的 `oneSdkConfig.config`; 协议版本或渠道参数不符时报协议错误.
- 账号凭据只写入桌面密钥环, 每个账号一个条目, 属性为
  `service + game + distribution + account_id`; 同一渠道用户重新登录沿用原账号编号.
- 国服能力门 `require_supported` 同时供宿主直接启动使用: 其他游戏或区服在
  接触账号数据前拒绝, 中文提示沿用宿主文案.
- 会话帧与查询回包是定长小端协议, 由游戏侧按官方协议解析.

## 依赖

- `oddity-platform`: 密钥环能力缝 (`SecretStore`: 生产 Secret Service, 测试内存实现),
  共享 HTTP 连接池与 User-Agent 常量.
- `oddity-runtime`: 命令组与宿主设置快照 (`HostContext::settings`,
  `HostContext::official_launcher_root`).
- `serde` / `serde_json` / `zeroize` / `tokio` / `aes` / `md-5` / `sha2`.
- 不依赖宿主实现, 只经 `oddity-runtime` 的能力缝.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte auth
```

网络路径用回环 HTTP 服务替代外网; 密钥环用 `MemorySecretStore`; 认证配置由
测试内的合成资源包提供; 会话帧与固定夹具逐字节对照. 测试完全离线.

## 限制

- 认证配置来自官方启动器安装目录, 未安装时只能看到"请先安装官方组件和游戏资源".
- 密钥环未启动或未解锁时账号命令全部不可用; 发送验证码前先检查密钥环.
- 直接启动的进程编排仍由宿主负责 (`oddity-host/src/session/host.rs`),
  包只提供会话与 SDK 查询.
