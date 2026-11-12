#!/bin/sh
set -eu
cd "$(dirname "$0")"
# winegcc -b x86_64-windows produces real PE32+ executables (not Wine ELF).
# 窗口守卫在本轮官方 UI 线程中阻止主窗口首次映射.
winegcc -b x86_64-windows -shared -O2 -Wall -Wextra -Werror -o nte-window-guard.dll nte-window-guard.c -lcomctl32
file nte-window-guard.dll
winegcc -b x86_64-windows -mwindows -O2 -Wall -Wextra -Werror -o nte-sdk-host.exe nte-sdk-host.c -lws2_32
file nte-sdk-host.exe
# 生产 helper 是 GUI 子系统进程: 由 umu-run 拉起时不能创建 Wine 控制台窗口.
for src in nte-launch-helper.c nte-session-helper.c; do
    exe=$(basename "$src" .c).exe
    echo "building $exe"
    winegcc -b x86_64-windows -mwindows -O2 -Wall -Wextra -Werror -o "$exe" "$src"
    file "$exe"
done
