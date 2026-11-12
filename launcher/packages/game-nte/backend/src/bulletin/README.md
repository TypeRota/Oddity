# 公告域

异环官方公告聚合: 抓取官网公告、版本前瞻 banner、B 站动态与 X 时间线，解析成
`BulletinFeed` 交给前端展示。每个来源独立失败, 调用方仍能展示其他来源。

## 职责

- 维护各官方来源的地址与解析规则 (`parse_announcement_html`,
  `parse_banner_script`, `parse_bilibili_json`, `parse_x_timeline`).
- 通过命令组声明并处理 `fetch_bulletins`, 参数只有区服 (`cn` / `global`).
- 区服在包内收敛为最小类型 `Region`, 不依赖宿主的完整游戏地区配置.

## 依赖

- `oddity-platform`: 错误类型、共享 HTTP 连接池与 User-Agent 常量.
- `oddity-runtime`: 命令组与载荷类型.
- `serde` / `serde_json`: 参数解析与返回结构.
- `tokio`: 把阻塞抓取移出异步线程.

## 测试入口

```
cd launcher && cargo test -p oddity-game-nte
```

解析规则用固定页面片段验证; 网络路径用回环 HTTP 服务替代外网, 测试完全离线.

## 限制

- 解析规则按当前官方页面结构编写, 页面改版时需要同步调整并补充样例.
- X 公开时间线在中国大陆可能不可达, 失败会作为来源错误返回.
- 抓取结果不缓存, 每次命令都重新请求各来源.
