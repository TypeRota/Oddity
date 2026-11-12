# 外部启动入口

Oddity 提供不需要先打开主界面的启动命令, 供应用菜单, 桌面快捷方式, Steam 游戏库或其他前端调用.

## 命令

```bash
oddity --launch
oddity --launch --game nte
oddity --launch --game nte --region cn
oddity --launch --game nte --region global
oddity --launch --game nte --region global --environment "GE-Proton 9"
oddity --launch --region cn --launch-mode official
oddity --launch --help
```

`--launch --help` 只打印用法, 不会启动游戏.

| 参数 | 说明 |
| --- | --- |
| `--launch` | 启动游戏, 必填. |
| `--game nte\|hsr` | 指定游戏. 省略时使用 `nte`. 当前只有《异环》支持启动, 其他游戏会提示尚未支持. |
| `--region <渠道 id>` | 指定游戏声明的发行渠道; 异环为 `cn` 或 `global`. 省略时使用该游戏当前设置中的渠道. |
| `--launch-mode official\|background\|direct` | 指定本次启动方式. 省略时固定使用 `background`. |
| `--environment <名称或 ID>` | 指定运行环境. 同时省略 `--region` 时沿用该游戏当前环境, 环境不可用时启动报错; 只指定 `--region` 时自动选择该地区可用的环境. |

规则:

- 启动目标解析成功后, `--game`, `--region` 和 `--environment` 会写回设置, 下次打开 Oddity 时仍是该游戏、地区和运行环境; 未支持启动的游戏不会写回.
- 不带 `--game` 的旧命令保持兼容, 默认按《异环》处理.
- 同时省略 `--region` 和 `--environment` 时沿用该游戏当前环境; 当前环境未设置或与启动条件不匹配时会报错, 不会自动切换环境.
- `--region` 必须是 `--game` 指定游戏声明的渠道.
- `--environment` 必须属于 `--game` 指定的游戏, 不能跨游戏组合.
- `--launch-mode` 只对本次启动生效, 不修改设置中的启动方式.
- 只给 `--region` 时, 优先使用当前环境; 其次使用稳定环境; 再次使用该地区唯一的匹配环境. 有多个候选时会报错并提示使用 `--environment`.
- 同时给出 `--region` 和 `--environment` 时, 两者必须属于同一渠道.
- 参数错误会在启动 Oddity 主程序前结束, 退出码为 2.

## 应用菜单

安装包会按游戏在 `oddity.desktop` 中注册动作:

| 动作 | 命令 |
| --- | --- |
| 异环 | `oddity --launch --game nte --region cn --launch-mode background` |
| NTE | `oddity --launch --game nte --region global --launch-mode background` |

动作位于 Oddity 图标的右键菜单或应用菜单的动作菜单中. 主入口 Oddity 仍用于打开启动器界面. 旧版本安装的桌面入口不带 `--game`, 仍然可以正常启动.

## XWayland 模式

在 Oddity 图标的动作菜单中选择 `XWayland 模式`, 或运行:

```bash
oddity --xwayland
```

此动作打开启动器界面, 当前实例及其游戏使用 X11/XWayland. 可以组合 `--launch` 参数直接启动游戏. 显示模式只影响本次 Oddity 进程, 不写入运行环境设置.

切换前请从托盘完全退出 Oddity. 已运行的 Wayland 实例无法通过单实例转发切换显示后端, 会提示退出后重试. 此动作不会创建独立的应用快捷方式.

游戏与官方 SDK 进程不继承 `proxy` 和 `*_proxy` 代理环境变量 (忽略大小写). Oddity 自身的下载与登录请求仍沿用原代理环境.

## Steam

1. 在 Steam 中添加非 Steam 游戏, 目标选择 `oddity`.
2. 在启动选项里填入 `--launch --game nte --region cn --launch-mode background`.
3. 国际服使用 `--launch --game nte --region global --launch-mode background`.
4. 旧的不带 `--game` 的启动选项仍然有效, 默认启动《异环》.

参数要放在启动选项里, 不要直接拼在可执行文件路径后面. Steam 的快捷方式名称可以写成 `异环` 或 `NTE`.

## 启动结果与错误

冷启动且启动成功时, Oddity 保持隐藏并常驻托盘; 游戏退出后按设置决定是否显示主界面.

启动失败时:

- Oddity 通过桌面通知显示 `异环启动失败` 或 `NTE 启动失败`, 正文是具体原因.
- 通知服务不可用时退回主界面提示, 并写入启动日志.
- 冷启动失败且通知发送成功时, 进程以退出码 1 结束, 不会留下空的托盘图标.
- 已经运行的 Oddity 收到转发请求时不会退出, 只显示通知.

转发请求的退出码固定为 0, 只表示参数已被主实例接收, 不表示游戏已经成功启动. 需要可靠结果时请查看桌面通知或启动日志.

## 已知限制

- 已运行 Oddity 时, `--launch` 会把请求转发给现有实例, 游戏不是 Steam 的子进程. Steam Overlay 和 Steam 运行时长在这条路径上不保证可用.
- 冷启动时 Oddity 作为 Steam 的子进程常驻, Steam 可能在游戏退出后仍认为快捷方式正在运行.
- Steam 的停止按钮不保证结束游戏进程组, 建议从游戏内正常退出.
- 没有 D-Bus 会话总线时, 单实例转发不可用, 可能启动第二个 Oddity.
- 更新 Oddity 后如果旧实例仍在运行, 新的桌面动作会转发给旧实例. 旧版本不认识 `--launch`, 只会显示主界面; 更新后请重启 Oddity.

## 日志

启动日志位于:

```text
~/.local/share/oddity/logs/
```

详细排查见[排障指南](troubleshooting.md).
