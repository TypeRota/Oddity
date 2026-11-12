# 塔吉多签到域

账号登录 (密码与短信), 每日签到任务, 游戏状态查询与自动执行设置.
所有入口共享同一份会话, 文件锁和执行结果.

## 职责

- 维护塔吉多与老虎账号协议: 登录, 刷新会话, 角色与社区签到, 金币任务, 云异环时长.
- 账号凭据只写入桌面密钥环, 单条目假设 (属性为 service + username);
  配置目录的 `state.json` 保存签到状态与任务授权, 不含任何凭据.
- 通过命令组声明并处理 `attendance_*`; 参数, 返回与错误载荷与宿主实现逐字段一致.
- 签到 run 是长任务: 交给宿主执行器, 进度按账号经 `task-progress` 事件上报,
  命令在任务结束后才返回; 成功, 失败与取消是三种不同结局.
- 每日与开机模式生成 systemd 用户单元和定时器, 单元用当前进程路径重启无头入口.
- 状态按北京时间自然日计算; 已完成的账号与任务在当天跳过, 附加任务单独重试.

## 依赖

- `oddity-platform`: 密钥环能力缝 (`SecretStore`: 生产 Secret Service, 测试内存实现),
  共享 HTTP 连接池, 时钟与错误类型.
- `oddity-runtime`: 命令组, 宿主设置快照与任务提交
  (`HostContext::settings`, `HostContext::submit`).
- `serde` / `serde_json` / `zeroize` / `tokio`: 载荷, 凭据清零与阻塞任务.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte attendance
```

网络路径用回环 HTTP 服务替代外网; 密钥环用 `MemorySecretStore`; 定时器写入
临时 `XDG_CONFIG_HOME`, 并用脚本替代 `systemctl`. 测试完全离线.

## 限制

- 塔吉多只提供国服账号服务; 国际服与其他游戏在接触账号数据前拒绝.
- 启动时签到与无头入口由宿主进程经 `PackageRuntime` 触发
  (`runtime.rs`): 包内不持有宿主的常驻进程生命周期.
- 自动执行依赖 systemd 用户定时器与已解锁的密钥环; 移动可执行文件后需要重新设置.
- 签到 run 不检查取消令牌: 任务句柄支持取消, 但进行中的 run 会跑到结束;
  账号之间靠文件锁串行, 与自动执行共用同一份状态.
