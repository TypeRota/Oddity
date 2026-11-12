//! 官方公告聚合。
//!
//! 页面由官方服务端生成，启动器只提取必要的元数据和跳转链接，避免在
//! WebView 中直接加载第三方页面。每个来源独立失败，调用方仍可展示其他来源。
//!
//! 区服只区分公告使用的两个官方站点，不依赖宿主的完整游戏地区配置；
//! 网络能力统一来自 oddity-platform。

use std::io::Read;
use std::time::Duration;

use oddity_platform::http::{status_code, HttpClient};
use oddity_platform::{Error, Result, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 公告域注册到宿主命令组里的命令 id。
pub(crate) const COMMANDS: &[&str] = &["fetch_bulletins"];

/// 公告来源使用的区服。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Region {
    #[default]
    Cn,
    Global,
}

pub const OFFICIAL_ANNOUNCEMENT_URL: &str = "https://yh.wanmei.com/news/gamebroad/index.html";
pub const OFFICIAL_LAUNCHER_URL: &str = "https://yh.wanmei.com/launcher/launcher_ob2.html?expand=0";
pub const OFFICIAL_SWIPER_URL: &str =
    "https://static.games.wanmei.com/public/commonData/gamesData/gameSwiper/yh-gameSwiper.js";
pub const X_PROFILE_URL: &str = "https://x.com/NTE_JP";
const X_TIMELINE_URL: &str =
    "https://syndication.twitter.com/srv/timeline-profile/screen-name/NTE_JP?showReplies=false";

pub const BILIBILI_SPACE_URL: &str = "https://space.bilibili.com/3546636978489848/dynamic";
pub const BILIBILI_API_URL: &str =
    "https://api.bilibili.com/x/polymer/web-dynamic/desktop/v1/feed/space?host_mid=3546636978489848&features=itemOpusStyle";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BulletinKind {
    Bilibili,
    X,
    Preview,
    Maintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BulletinItem {
    pub kind: BulletinKind,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub image_url: Option<String>,
    pub published_at: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BulletinFeed {
    pub items: Vec<BulletinItem>,
    pub errors: Vec<String>,
}

const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/131.0 Safari/537.36";

fn fetch_text(url: &str, limit: u64, referer: Option<&str>) -> Result<String> {
    let timeout = Duration::from_secs(if url == BILIBILI_API_URL { 8 } else { 15 });
    let user_agent = referer.map_or(USER_AGENT, |_| BROWSER_USER_AGENT);
    let mut request = HttpClient::shared()
        .request_as(url, timeout, user_agent)
        .header(
            "Accept",
            if url == BILIBILI_API_URL {
                "application/json, text/plain, */*"
            } else {
                "text/html,application/json,*/*"
            },
        );
    if let Some(referer) = referer {
        request = request.header("Referer", referer);
    }
    let mut response = request
        .call()
        .map_err(|error| Error::Http(format!("GET {url}: {error}")))?;
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(limit)
        .read_to_string(&mut body)
        .map_err(|error| Error::Http(format!("read {url}: {error}")))?;
    Ok(body)
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn strip_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    decode_entities(&output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn class_text(block: &str, class_name: &str) -> String {
    let marker = format!("class=\"{class_name}\"");
    let Some(class_start) = block.find(&marker) else {
        return String::new();
    };
    let Some(start) = block[class_start..]
        .find('>')
        .map(|index| class_start + index + 1)
    else {
        return String::new();
    };
    let Some(end) = block[start..].find('<').map(|index| start + index) else {
        return String::new();
    };
    strip_tags(&block[start..end])
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// 解析公告列表页，按官方页面顺序保留最新的三条公告。
pub fn parse_announcement_html(html: &str) -> Vec<BulletinItem> {
    parse_official_list(
        html,
        "https://yh.wanmei.com",
        "/news/gamebroad/",
        BulletinKind::Maintenance,
    )
}

fn parse_official_list(
    html: &str,
    origin: &str,
    channel: &str,
    kind: BulletinKind,
) -> Vec<BulletinItem> {
    let mut items = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = html[cursor..].find("<a ") {
        let start = cursor + relative;
        let Some(tag_end) = html[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_end;
        let Some(close_relative) = html[tag_end..].find("</a>") else {
            break;
        };
        let end = tag_end + close_relative + 4;
        let tag = &html[start..=tag_end];
        let block = &html[start..end];
        let Some(href) = attr(tag, "href") else {
            cursor = end;
            continue;
        };
        if !href.contains(channel) {
            cursor = end;
            continue;
        }
        let title = class_text(block, "title");
        if title.is_empty() {
            cursor = end;
            continue;
        }
        items.push(BulletinItem {
            kind: kind.clone(),
            title,
            summary: class_text(block, "des"),
            url: if href.starts_with("https://") || href.starts_with("http://") {
                href
            } else {
                format!("{origin}/{}", href.trim_start_matches('/'))
            },
            image_url: block
                .find("<img ")
                .and_then(|start| {
                    let tag = &block[start..];
                    attr(tag.split_once('>')?.0, "src")
                })
                .filter(|url| !url.is_empty())
                .map(|url| {
                    if url.starts_with("//") {
                        format!("https:{url}")
                    } else if url.starts_with('/') {
                        format!("{origin}{url}")
                    } else {
                        url
                    }
                }),
            published_at: class_text(block, "date"),
            source: "官方公告".to_string(),
        });
        if items.len() >= 3 {
            break;
        }
        cursor = end;
    }
    items
}

/// 解析 `yh-gameSwiper.js` 中的 `yh_data_data`，只取最新一张前瞻 banner。
pub fn parse_banner_script(script: &str) -> Result<Vec<BulletinItem>> {
    let start = script
        .find("var yh_data_data")
        .and_then(|index| script[index..].find('=').map(|offset| index + offset + 1))
        .ok_or_else(|| Error::Parse("yh-gameSwiper.js 缺少 yh_data_data".to_string()))?;
    let json = script[start..]
        .split_once(';')
        .map(|(value, _)| value.trim())
        .unwrap_or_else(|| script[start..].trim());
    let root: Value = serde_json::from_str(json)
        .map_err(|error| Error::Parse(format!("解析 yh_data_data 失败：{error}")))?;
    let Some(entry) = root.get("lb1").and_then(Value::as_array).and_then(|items| {
        items.iter().find(|item| {
            item.get("title")
                .and_then(Value::as_str)
                .is_some_and(|title| !title.is_empty())
                && item
                    .get("link")
                    .and_then(Value::as_str)
                    .is_some_and(|url| !url.is_empty())
        })
    }) else {
        return Ok(Vec::new());
    };
    let title = entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let url = entry
        .get("link")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if title.is_empty() || url.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![BulletinItem {
        kind: BulletinKind::Preview,
        title: title.to_string(),
        summary: "版本前瞻回顾".to_string(),
        url: url.to_string(),
        image_url: entry
            .get("bigpic")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        published_at: String::new(),
        source: "版本前瞻".to_string(),
    }])
}

fn value_text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn module_text(module: &Value) -> String {
    let text = value_text(module.get("text"));
    if !text.is_empty() {
        return text;
    }
    module
        .get("rich_text_nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn split_dynamic_text(text: &str) -> (String, String) {
    let title = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("官方最新动态")
        .to_string();
    let summary = text
        .strip_prefix(&title)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(text)
        .to_string();
    (title, summary)
}

fn module<'a>(modules: &'a Value, name: &str) -> Option<&'a Value> {
    modules
        .get(name)
        .or_else(|| modules.as_array()?.iter().find_map(|item| item.get(name)))
}

fn contains_lottery(value: &Value) -> bool {
    match value {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            let key_is_lottery =
                key.eq_ignore_ascii_case("lottery") || key.eq_ignore_ascii_case("lottery_id");
            let value_is_lottery = value.as_str().is_some_and(|text| {
                text.eq_ignore_ascii_case("RICH_TEXT_NODE_TYPE_LOTTERY")
                    || text.contains("抽奖")
                    || text.contains("中奖")
            });
            key_is_lottery || value_is_lottery || contains_lottery(value)
        }),
        Value::Array(values) => values.iter().any(contains_lottery),
        _ => false,
    }
}

/// 图片地址统一改用 HTTPS, 避免被界面 CSP 与混合内容策略拦截.
fn secure_image_url(url: &str) -> String {
    url.strip_prefix("http://")
        .map_or_else(|| url.to_string(), |rest| format!("https://{rest}"))
}

/// 解析 B 站空间动态 API。接口可能因风控返回错误码，此时返回可展示的解析错误。
pub fn parse_bilibili_json(body: &str) -> Result<Vec<BulletinItem>> {
    let root: Value = serde_json::from_str(body)
        .map_err(|error| Error::Parse(format!("解析 B 站动态失败：{error}")))?;
    let code = root.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let message = value_text(root.get("message"));
        return Err(Error::Parse(format!("B 站接口返回 {code}: {message}")));
    }
    let Some(items) = root
        .get("data")
        .and_then(|data| data.get("items"))
        .and_then(Value::as_array)
    else {
        return Err(Error::Parse("B 站接口缺少 data.items".to_string()));
    };
    let mut output = Vec::new();
    for item in items {
        if contains_lottery(item) {
            continue;
        }
        let modules = item.get("modules").unwrap_or(&Value::Null);
        let dynamic = module(modules, "module_dynamic").unwrap_or(&Value::Null);
        let archive = dynamic
            .get("major")
            .and_then(|major| major.get("archive"))
            .or_else(|| dynamic.get("dyn_archive"));
        let description = module(modules, "module_desc")
            .map(module_text)
            .filter(|text| !text.is_empty())
            .or_else(|| dynamic.get("desc").map(module_text))
            .unwrap_or_default();
        let archive_title = value_text(archive.and_then(|value| value.get("title")));
        let archive_description = value_text(archive.and_then(|value| value.get("desc")));
        let (title, summary) = if !archive_title.is_empty() {
            (
                archive_title,
                if archive_description.is_empty() {
                    description
                } else {
                    archive_description
                },
            )
        } else {
            split_dynamic_text(&description)
        };
        let id = value_text(item.get("id_str"));
        let url = if id.is_empty() {
            BILIBILI_SPACE_URL.to_string()
        } else {
            format!("https://t.bilibili.com/{id}")
        };
        let image_url = archive
            .and_then(|v| v.get("cover"))
            .and_then(Value::as_str)
            .or_else(|| {
                dynamic
                    .get("major")
                    .and_then(|major| major.get("draw"))
                    .and_then(|draw| draw.get("items"))
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|image| image.get("src"))
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                dynamic
                    .get("dyn_draw")
                    .and_then(|draw| draw.get("items"))
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|image| image.get("src"))
                    .and_then(Value::as_str)
            })
            .filter(|value| !value.is_empty())
            .map(secure_image_url);
        let author = module(modules, "module_author").unwrap_or(&Value::Null);
        let published_at = {
            let value = value_text(author.get("pub_time"));
            if value.is_empty() {
                value_text(author.get("pub_text"))
            } else {
                value
            }
        };
        let timestamp = author.get("pub_ts").and_then(Value::as_i64).unwrap_or(0);
        output.push((
            timestamp,
            BulletinItem {
                kind: BulletinKind::Bilibili,
                title,
                summary,
                url,
                image_url,
                published_at,
                source: "B站官方动态".to_string(),
            },
        ));
    }
    output.sort_by_key(|item| std::cmp::Reverse(item.0));
    Ok(output.into_iter().take(5).map(|(_, item)| item).collect())
}

/// 公开嵌入时间线与官网公告分别读取, 单个来源失败不影响其他来源。
fn fetch_x_text() -> Result<String> {
    let request = HttpClient::shared()
        .request_as(X_TIMELINE_URL, Duration::from_secs(12), BROWSER_USER_AGENT)
        .header("Accept", "text/html")
        .header("Referer", "https://platform.twitter.com/");
    let mut response = request.call().map_err(|error| {
        Error::Http(match status_code(&error) {
            Some(429) => "X 暂时拒绝请求或限制访问, 请稍后重试.".to_string(),
            Some(403) => "X 拒绝公开时间线请求, 可打开官方账号查看.".to_string(),
            Some(code) => format!("X 服务返回 HTTP {code}, 请稍后重试."),
            None => "无法连接 X. 在中国大陆可能需要使用可访问 X 的网络连接.".to_string(),
        })
    })?;
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .take(2 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|_| Error::Http("读取 X 动态超时或连接中断, 请重试.".to_string()))?;
    Ok(body)
}

pub fn parse_x_timeline(html: &str) -> Result<Vec<BulletinItem>> {
    let start = html
        .find("id=\"__NEXT_DATA__\"")
        .and_then(|start| html[start..].find('>').map(|offset| start + offset + 1))
        .ok_or_else(|| Error::Parse("X 未返回公开动态数据.".to_string()))?;
    let end = html[start..]
        .find("</script>")
        .map(|offset| start + offset)
        .ok_or_else(|| Error::Parse("X 动态数据不完整.".to_string()))?;
    let root: Value = serde_json::from_str(&html[start..end])
        .map_err(|_| Error::Parse("X 动态数据格式已变化.".to_string()))?;
    let entries = root
        .pointer("/props/pageProps/timeline/entries")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Parse("X 未提供公开时间线, 可打开官方账号查看.".to_string()))?;
    let mut items = Vec::new();
    for entry in entries {
        if entry.get("type").and_then(Value::as_str) != Some("tweet") {
            continue;
        }
        let Some(tweet) = entry.pointer("/content/tweet") else {
            continue;
        };
        if !value_text(tweet.pointer("/user/screen_name")).eq_ignore_ascii_case("NTE_JP") {
            continue;
        }
        let id = value_text(tweet.get("id_str"));
        let text = value_text(tweet.get("full_text").or_else(|| tweet.get("text")));
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) || text.is_empty() {
            continue;
        }
        let (title, summary) = split_dynamic_text(&decode_entities(&text));
        items.push(BulletinItem {
            kind: BulletinKind::X,
            title,
            summary,
            url: format!("{X_PROFILE_URL}/status/{id}"),
            image_url: tweet
                .pointer("/extended_entities/media/0/media_url_https")
                .or_else(|| tweet.pointer("/entities/media/0/media_url_https"))
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(str::to_string),
            published_at: value_text(tweet.get("created_at")),
            source: "X / NTE_JP".to_string(),
        });
        if items.len() == 5 {
            break;
        }
    }
    Ok(items)
}

fn retry_bilibili(
    mut fetch: impl FnMut() -> Result<Vec<BulletinItem>>,
    delay: Duration,
) -> Result<Vec<BulletinItem>> {
    let first = fetch();
    if matches!(&first, Ok(items) if !items.is_empty()) {
        return first;
    }
    std::thread::sleep(delay);
    let items = fetch()?;
    if items.is_empty() {
        return Err(Error::Parse("B 站未返回可用动态, 请稍后重试.".to_string()));
    }
    Ok(items)
}

fn collect(
    feed: &mut BulletinFeed,
    label: &str,
    result: std::thread::Result<Result<Vec<BulletinItem>>>,
) {
    match result {
        Ok(Ok(items)) => feed.items.extend(items),
        Ok(Err(error)) => {
            let message = match error {
                Error::Http(message) | Error::Parse(message) => message,
                error => error.to_string(),
            };
            feed.errors.push(format!("{label}: {message}"));
        }
        Err(_) => feed
            .errors
            .push(format!("{label}: 抓取任务异常结束, 请重试.")),
    }
}

pub(crate) fn fetch_for(region: Region) -> BulletinFeed {
    fetch_sources(region, |url, limit, referer| {
        if url == X_TIMELINE_URL {
            fetch_x_text()
        } else {
            fetch_text(url, limit, referer)
        }
    })
}

// 网络读取独立于并发聚合, 便于用固定响应验证来源隔离与输出顺序。
fn fetch_sources(
    region: Region,
    fetch: impl Fn(&str, u64, Option<&str>) -> Result<String> + Sync,
) -> BulletinFeed {
    let cn = region == Region::Cn;
    std::thread::scope(|scope| {
        let social = scope.spawn(|| {
            if cn {
                retry_bilibili(
                    || {
                        fetch(BILIBILI_API_URL, 2 * 1024 * 1024, Some(BILIBILI_SPACE_URL))
                            .and_then(|body| parse_bilibili_json(&body))
                    },
                    Duration::from_millis(900),
                )
            } else {
                fetch(
                    X_TIMELINE_URL,
                    2 * 1024 * 1024,
                    Some("https://platform.twitter.com/"),
                )
                .and_then(|body| parse_x_timeline(&body))
            }
        });
        let news = scope.spawn(|| {
            if cn {
                fetch(OFFICIAL_SWIPER_URL, 256 * 1024, Some(OFFICIAL_LAUNCHER_URL))
                    .and_then(|body| parse_banner_script(&body))
            } else {
                let origin = "https://nte.perfectworld.com";
                let url = format!("{origin}/en/article/news/gamenews/index.html");
                fetch(&url, 1024 * 1024, Some(&url)).map(|html| {
                    parse_official_list(
                        &html,
                        origin,
                        "/en/article/news/gamenews/",
                        BulletinKind::Preview,
                    )
                })
            }
        });
        let maintenance = scope.spawn(|| {
            if cn {
                fetch(
                    OFFICIAL_ANNOUNCEMENT_URL,
                    1024 * 1024,
                    Some(OFFICIAL_ANNOUNCEMENT_URL),
                )
                .map(|body| parse_announcement_html(&body))
            } else {
                let origin = "https://nte.perfectworld.com";
                let url = format!("{origin}/en/article/news/gamebroad/index.html");
                fetch(&url, 1024 * 1024, Some(&url)).map(|html| {
                    parse_official_list(
                        &html,
                        origin,
                        "/en/article/news/gamebroad/",
                        BulletinKind::Maintenance,
                    )
                })
            }
        });
        let mut feed = BulletinFeed::default();
        collect(
            &mut feed,
            if cn { "B站动态" } else { "X / NTE_JP" },
            social.join(),
        );
        collect(
            &mut feed,
            if cn {
                "版本前瞻"
            } else {
                "国际服新闻"
            },
            news.join(),
        );
        collect(
            &mut feed,
            if cn {
                "停服公告"
            } else {
                "国际服公告"
            },
            maintenance.join(),
        );
        feed
    })
}

/// 公告命令的 JSON 载荷: 与契约里的参数形状保持一致.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    distribution: Region,
}

/// 分发公告域命令; 参数与结果都是宿主载荷。
pub(crate) async fn command(command: &str, args: Value) -> std::result::Result<Value, Value> {
    match command {
        "fetch_bulletins" => fetch_bulletins(args).await,
        _ => Err(json!("无法识别的公告命令")),
    }
}

/// 抓取入口与旧的宿主实现保持一致: 阻塞抓取移出异步线程, 序列化失败按字符串返回。
async fn fetch_bulletins(args: Value) -> std::result::Result<Value, Value> {
    let Args { distribution } =
        serde_json::from_value(args).map_err(|error| json!(error.to_string()))?;
    let feed = tokio::task::spawn_blocking(move || fetch_for(distribution))
        .await
        .map_err(|error| json!(format!("公告抓取任务失败：{error}")))?;
    serde_json::to_value(feed).map_err(|error| json!(error.to_string()))
}

#[cfg(test)]
mod tests;
