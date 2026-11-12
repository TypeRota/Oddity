//! 公告抓取与解析测试。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use oddity_platform::Error;
use serde_json::json;

use super::*;

#[test]
fn parses_announcement_list_and_limits_to_three() {
    let html = r#"<div class="listNews"><a href="/news/gamebroad/20260101/a.html"><div class="listItem"><h2 class="title">第一条</h2><div class="des">摘要&nbsp;内容</div><p class="date">2026-01-01</p></div></a><a href="/news/gamebroad/20260102/b.html"><div class="title">第二条</div><p class="date">2026-01-02</p></a><a href="/news/gamebroad/20260103/c.html"><h2 class="title">第三条</h2><p class="date">2026-01-03</p></a><a href="/news/gamebroad/20260104/d.html"><h2 class="title">第四条</h2></a></div>"#;
    let items = parse_announcement_html(html);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].title, "第一条");
    assert_eq!(items[0].summary, "摘要 内容");
    assert_eq!(
        items[0].url,
        "https://yh.wanmei.com/news/gamebroad/20260101/a.html"
    );
}

#[test]
fn global_announcements_keep_global_links_and_ignore_cn_entries() {
    let html = r#"<a href="/en/article/news/gamebroad/20261019/264665.html"><h2 class="title">Account Action Notice</h2><p class="date">2026-10-19</p></a><a href="https://yh.wanmei.com/news/gamebroad/a.html"><h2 class="title">国服公告</h2></a>"#;
    let items = parse_official_list(
        html,
        "https://nte.perfectworld.com",
        "/en/article/news/gamebroad/",
        BulletinKind::Maintenance,
    );
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].url,
        "https://nte.perfectworld.com/en/article/news/gamebroad/20261019/264665.html"
    );
    assert_eq!(items[0].published_at, "2026-10-19");
}

#[test]
fn parses_only_latest_banner() {
    let script = r#"var yh_data_data = {"lb1":[{"title":"最新前瞻","bigpic":"https://img.test/a.jpg","link":"https://yh.wanmei.com/a.html"},{"title":"旧前瞻","bigpic":"","link":"https://yh.wanmei.com/b.html"}]};"#;
    let items = parse_banner_script(script).expect("banner");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "最新前瞻");
}

#[test]
fn parses_bilibili_archive_and_text_dynamic() {
    let body = r#"{"code":0,"data":{"items":[{"id_str":"123","modules":{"module_author":{"pub_time":"刚刚"},"module_dynamic":{"desc":{"text":"官方文字动态"}}}},{"id_str":"456","modules":{"module_author":{"pub_time":"昨天"},"module_dynamic":{"desc":{"text":"视频说明"},"major":{"archive":{"title":"视频标题","cover":"https://img.test/v.jpg"}}}}}]}}"#;
    let items = parse_bilibili_json(body).expect("dynamic");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "官方文字动态");
    assert_eq!(items[0].summary, "官方文字动态");
    assert_eq!(items[1].title, "视频标题");
    assert_eq!(items[1].summary, "视频说明");
    assert_eq!(
        items[1].image_url.as_deref(),
        Some("https://img.test/v.jpg")
    );
}

#[test]
fn parses_desktop_bilibili_module_arrays() {
    let body = r#"{"code":0,"data":{"items":[{"id_str":"789","modules":[{"module_author":{"pub_time":"今天"}},{"module_dynamic":{"desc":{"text":"桌面接口动态"}}}]}]}}"#;
    let items = parse_bilibili_json(body).expect("desktop dynamic");
    assert_eq!(items[0].title, "桌面接口动态");
    assert_eq!(items[0].summary, "桌面接口动态");
    assert_eq!(items[0].published_at, "今天");
}

#[test]
fn parses_desktop_archive_fields() {
    let body = r#"{"code":0,"data":{"items":[{"id_str":"789","modules":[{"module_author":{"pub_text":"10月15日 · 投稿了视频"}},{"module_dynamic":{"dyn_archive":{"title":"视频标题","desc":"视频说明","cover":"https://img.test/v.jpg"}}}]}]}}"#;
    let items = parse_bilibili_json(body).expect("desktop archive");
    assert_eq!(items[0].title, "视频标题");
    assert_eq!(items[0].summary, "视频说明");
    assert_eq!(items[0].published_at, "10月15日 · 投稿了视频");
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://img.test/v.jpg")
    );
}

#[test]
fn bilibili_image_urls_use_https() {
    let body = r#"{"code":0,"data":{"items":[{"id_str":"789","modules":[{"module_author":{"pub_text":"今天"}},{"module_dynamic":{"dyn_archive":{"title":"视频标题","cover":"http://i0.hdslb.com/bfs/archive/a.jpg"}}}]}]}}"#;
    let items = parse_bilibili_json(body).expect("desktop archive");
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://i0.hdslb.com/bfs/archive/a.jpg")
    );
}

#[test]
fn filters_lottery_dynamics() {
    let body = r#"{"code":0,"data":{"items":[{"id_str":"1260576105436282904","type":"DYNAMIC_TYPE_FORWARD","modules":[{"module_desc":{"rich_text_nodes":[{"type":"RICH_TEXT_NODE_TYPE_LOTTERY","text":"互动抽奖"}]}},{"module_author":{"pub_text":"3天前"}}]},{"id_str":"789","modules":[{"module_author":{"pub_text":"今天"}},{"module_dynamic":{"desc":{"text":"普通动态"}}}]}]}}"#;
    let items = parse_bilibili_json(body).expect("dynamic");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url, "https://t.bilibili.com/789");
}

#[test]
fn parses_desktop_text_module_for_title_and_content() {
    let body = r##"{"code":0,"data":{"items":[{"id_str":"789","modules":[{"module_author":{"pub_text":"今天"}},{"module_desc":{"text":"#异环#\n感谢鉴定师们的一路同行~"}},{"module_dynamic":{"dyn_draw":{"items":[{"src":"https://img.test/draw.jpg"}]}}}]}]}}"##;
    let items = parse_bilibili_json(body).expect("dynamic");
    assert_eq!(items[0].title, "#异环#");
    assert_eq!(items[0].summary, "感谢鉴定师们的一路同行~");
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://img.test/draw.jpg")
    );
}
#[test]
fn official_news_accepts_image_and_text_only_entries() {
    let html = r#"<a href="/news/a"><h2 class="title">配图新闻</h2><img src="//images.test/news.jpg"></a><a href="/news/b"><h2 class="title">文字新闻</h2></a><a href="/news/c"><h2 class="title">站内配图</h2><img src="/images/news.jpg"></a>"#;
    let items = parse_official_list(
        html,
        "https://official.test",
        "/news/",
        BulletinKind::Preview,
    );
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://images.test/news.jpg")
    );
    assert_eq!(items[1].image_url, None);
    assert_eq!(
        items[2].image_url.as_deref(),
        Some("https://official.test/images/news.jpg")
    );
}

#[test]
fn x_timeline_keeps_official_photo_and_text_posts() {
    let root = serde_json::json!({"props":{"pageProps":{"timeline":{"entries":[
        {"type":"tweet","content":{"tweet":{"id_str":"123","full_text":"お知らせ\nNTE &amp; 新情報","user":{"screen_name":"NTE_JP"},"extended_entities":{"media":[{"media_url_https":"https://pbs.twimg.com/media/photo.jpg"}]}}}},
        {"type":"tweet","content":{"tweet":{"id_str":"456","text":"文字のお知らせ","user":{"screen_name":"nte_jp"}}}},
        {"type":"tweet","content":{"tweet":{"id_str":"789","text":"別のアカウント","user":{"screen_name":"other"}}}},
        {"type":"tweet","content":{"tweet":{"id_str":"invalid/id","text":"不正なリンク","user":{"screen_name":"NTE_JP"}}}}
    ]}}}});
    let html = format!("<script id=\"__NEXT_DATA__\" type=\"application/json\">{root}</script>");
    let items = parse_x_timeline(&html).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, BulletinKind::X);
    assert_eq!(items[0].title, "お知らせ");
    assert_eq!(items[0].summary, "NTE & 新情報");
    assert_eq!(items[0].url, "https://x.com/NTE_JP/status/123");
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://pbs.twimg.com/media/photo.jpg")
    );
    assert_eq!(items[1].image_url, None);
    assert!(parse_x_timeline("<html>Sign in</html>").is_err());
    assert!(parse_x_timeline(r#"<script id="__NEXT_DATA__">{}</script>"#).is_err());
}

#[test]
fn bilibili_retries_transient_errors_and_empty_responses_once() {
    for empty in [false, true] {
        let mut calls = 0;
        let result = retry_bilibili(|| {
            calls += 1;
            if calls == 1 {
                if empty { Ok(vec![]) } else { Err(Error::Http("temporary".into())) }
            } else {
                parse_bilibili_json(r#"{"code":0,"data":{"items":[{"id_str":"123","modules":{"module_dynamic":{"desc":{"text":"官方动态"}}}}]}}"#)
            }
        }, Duration::ZERO).unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result[0].title, "官方动态");
    }
    let mut calls = 0;
    let result = retry_bilibili(
        || {
            calls += 1;
            Ok(vec![])
        },
        Duration::ZERO,
    );
    assert_eq!(calls, 2);
    assert!(result.unwrap_err().to_string().contains("未返回可用动态"));
    let mut calls = 0;
    let result = retry_bilibili(
        || {
            calls += 1;
            Err(Error::Http("offline".into()))
        },
        Duration::ZERO,
    );
    assert_eq!(calls, 2);
    assert!(result.unwrap_err().to_string().contains("offline"));
}
#[test]
fn fetch_text_limits_response_and_reports_http_and_decode_errors() {
    let server = TestServer::start();
    server.route("/news", "official news");
    server.route("/invalid", [0xff]);
    assert_eq!(
        fetch_text(&server.url("/news"), 8, None).unwrap(),
        "official"
    );
    assert_eq!(
        fetch_text(&server.url("/news"), 1024, Some(OFFICIAL_LAUNCHER_URL)).unwrap(),
        "official news"
    );
    assert!(fetch_text(&server.url("/missing"), 1024, None)
        .unwrap_err()
        .to_string()
        .contains("GET"));
    assert!(fetch_text(&server.url("/invalid"), 1024, None)
        .unwrap_err()
        .to_string()
        .contains("read"));
}

#[test]
fn collect_preserves_successful_sources_and_labels_each_failure() {
    let mut feed = BulletinFeed::default();
    let items =
        parse_announcement_html(r#"<a href="/news/gamebroad/a"><h2 class="title">公告</h2></a>"#);
    assert_eq!(items.len(), 1);
    collect(&mut feed, "官网", Ok(Ok(items.clone())));
    collect(&mut feed, "网络", Ok(Err(Error::Http("offline".into()))));
    collect(&mut feed, "格式", Ok(Err(Error::Parse("invalid".into()))));
    let io = Error::Io(std::io::Error::other("disk"));
    let message = io.to_string();
    collect(&mut feed, "其他", Ok(Err(io)));
    collect(&mut feed, "任务", Err(Box::new("panic")));
    assert_eq!(feed.items, items);
    assert_eq!(
        feed.errors,
        [
            "网络: offline".to_string(),
            "格式: invalid".to_string(),
            format!("其他: {message}"),
            "任务: 抓取任务异常结束, 请重试.".to_string()
        ]
    );
}

#[test]
fn bilibili_success_does_not_retry() {
    let mut calls = 0;
    let items = retry_bilibili(
        || {
            calls += 1;
            Ok(parse_announcement_html(
                r#"<a href="/news/gamebroad/a"><h2 class="title">公告</h2></a>"#,
            ))
        },
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(items.len(), 1);
}

#[test]
fn x_timeline_rejects_incomplete_data_and_limits_valid_posts() {
    assert!(parse_x_timeline(r#"<script id="__NEXT_DATA__">"#)
        .unwrap_err()
        .to_string()
        .contains("不完整"));
    assert!(
        parse_x_timeline(r#"<script id="__NEXT_DATA__">invalid</script>"#)
            .unwrap_err()
            .to_string()
            .contains("格式")
    );
    let mut entries = vec![
        serde_json::json!({"type":"cursor"}),
        serde_json::json!({"type":"tweet"}),
    ];
    entries.extend((1..=6).map(|id| serde_json::json!({"type":"tweet","content":{"tweet":{"id_str":id.to_string(),"text":"official post","user":{"screen_name":"NTE_JP"},"entities":{"media":[{"media_url_https":"https://image.test/photo.jpg"}]}}}})));
    let root = serde_json::json!({"props":{"pageProps":{"timeline":{"entries":entries}}}});
    let items = parse_x_timeline(&format!("<script id=\"__NEXT_DATA__\">{root}</script>")).unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[4].url, "https://x.com/NTE_JP/status/5");
    assert_eq!(
        items[0].image_url.as_deref(),
        Some("https://image.test/photo.jpg")
    );
}
#[test]
fn sources_keep_distribution_order_and_survive_social_failure() {
    for region in [Region::Cn, Region::Global] {
        for fail_social in [false, true] {
            let feed = fetch_sources(region, |url, limit, referer| {
                assert!(limit > 0);
                assert!(referer.is_some());
                if url == BILIBILI_API_URL || url == X_TIMELINE_URL {
                    if fail_social {
                        return Err(Error::Http("offline".into()));
                    }
                    if url == BILIBILI_API_URL {
                        return Ok(r#"{"code":0,"data":{"items":[{"id_str":"123","modules":{"module_dynamic":{"desc":{"text":"动态"}}}}]}}"#.into());
                    }
                    return Ok(r#"<script id="__NEXT_DATA__">{"props":{"pageProps":{"timeline":{"entries":[{"type":"tweet","content":{"tweet":{"id_str":"123","text":"post","user":{"screen_name":"NTE_JP"}}}}]}}}}</script>"#.into());
                }
                if url == OFFICIAL_SWIPER_URL {
                    return Ok(r#"var yh_data_data = {"lb1":[{"title":"前瞻","bigpic":"","link":"https://yh.wanmei.com/a.html"}]};"#.into());
                }
                let path = if url == OFFICIAL_ANNOUNCEMENT_URL {
                    "/news/gamebroad/"
                } else if url.contains("gamenews") {
                    "/en/article/news/gamenews/"
                } else {
                    "/en/article/news/gamebroad/"
                };
                Ok(format!(
                    "<a href=\"{path}a.html\"><h2 class=\"title\">公告</h2></a>"
                ))
            });
            let mut expected = vec![
                if region == Region::Cn {
                    BulletinKind::Bilibili
                } else {
                    BulletinKind::X
                },
                BulletinKind::Preview,
                BulletinKind::Maintenance,
            ];
            if fail_social {
                expected.remove(0);
                assert_eq!(
                    feed.errors,
                    [format!(
                        "{}: offline",
                        if region == Region::Cn {
                            "B站动态"
                        } else {
                            "X / NTE_JP"
                        }
                    )]
                );
            } else {
                assert!(feed.errors.is_empty());
            }
            assert_eq!(
                feed.items
                    .iter()
                    .map(|item| item.kind.clone())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

/// 只服务已注册路径的回环 HTTP 服务, 让抓取测试完全离线.
struct TestServer {
    addr: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
}

impl TestServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定测试服务端口");
        let addr = listener.local_addr().expect("读取监听地址");
        listener.set_nonblocking(true).expect("设置非阻塞");
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_routes = Arc::clone(&routes);
        let worker_stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let routes = Arc::clone(&worker_routes);
                        thread::spawn(move || {
                            let _ = serve(stream, &routes);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self { addr, routes, stop }
    }

    fn route(&self, path: &str, body: impl AsRef<[u8]>) {
        self.routes
            .lock()
            .expect("锁定服务状态")
            .insert(path.to_string(), body.as_ref().to_vec());
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn serve(mut stream: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end().is_empty() {
            break;
        }
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_string();
    let body = routes.lock().expect("锁定服务状态").get(&path).cloned();
    match body {
        Some(body) => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes())?;
            stream.write_all(&body)?;
        }
        None => stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?,
    }
    stream.flush()
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("构建测试运行时")
        .block_on(future)
}

#[test]
fn region_only_accepts_the_two_official_markets() {
    assert_eq!(
        serde_json::from_value::<Region>(json!("cn")).unwrap(),
        Region::Cn
    );
    assert_eq!(
        serde_json::from_value::<Region>(json!("global")).unwrap(),
        Region::Global
    );
    let error = serde_json::from_value::<Region>(json!("eu"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("cn") && error.contains("global"), "{error}");
}

#[test]
fn command_rejects_unknown_ids_and_malformed_arguments() {
    let error = block_on(command("game_status", json!({}))).unwrap_err();
    assert_eq!(error, json!("无法识别的公告命令"));
    let error = block_on(command("fetch_bulletins", json!({}))).unwrap_err();
    assert!(error.as_str().unwrap().contains("missing field"), "{error}");
    let error = block_on(command("fetch_bulletins", json!({"distribution": "eu"}))).unwrap_err();
    assert!(
        error.as_str().unwrap().contains("unknown variant"),
        "{error}"
    );
    let error = block_on(command(
        "fetch_bulletins",
        json!({"distribution": "cn", "region": "cn"}),
    ))
    .unwrap_err();
    assert!(error.as_str().unwrap().contains("unknown field"), "{error}");
}
