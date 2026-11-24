# 跨屏游玩

设置中开启「跨屏游玩」后, 本机 Sunshine 会登记当前游戏. 在手机, 平板或电视上用 Moonlight 选择该条目即可开始, 分辨率和帧率随客户端自动适配.

## 前提

- KDE Plasma Wayland 会话.
- 已安装 Sunshine 与 Gamescope.
- Sunshine 的 `capture` 为 `kwin`, 且 `output_name` 指向当前屏幕; 留空时使用主屏.
- 已在 Sunshine 中设置过 Web 管理界面的用户名和密码, Moonlight 配对后才能请求应用列表.

缺少组件时设置页会给出安装入口, 用 `pkexec` 调用系统包管理器.

## 行为

- 开关打开时把条目写入 Sunshine 的 `apps.json`, 提交一次 `restart` 让它重新载入; 关闭时移除条目.
- 条目只有一条, 名字为 `Oddity - <游戏名>`. 已存在同名但命令不同的条目时不会覆盖, 需要先重命名.
- 串流开始时按 Moonlight 客户端请求的分辨率切换捕获屏幕的显示模式; 客户端分辨率没有对应模式时保持当前模式, 由 Gamescope 缩放.
- 串流结束时恢复切换前的模式, 缩放, 位置与主屏优先级. 恢复失败会保留记录, 设置页提供「恢复显示设置」.
- 分辨率和帧率只覆盖本次启动, 不写入游戏设置.

## 命令

Sunshine 注册的条目调用同一个入口, 也可以手动执行:

```bash
nte-cli cross-screen run --game nte
nte-cli cross-screen prepare --game nte
nte-cli cross-screen restore
```

`prepare` 由 Sunshine 的 prep-cmd 调用, `run` 是前台主命令, 两者都从 Sunshine 注入的环境变量读取客户端分辨率.
