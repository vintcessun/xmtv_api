use anyhow::{Context, Result, anyhow};
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::Url;

/// 整个进程共用一个 `Client`，复用连接池，避免每次请求都新建 TLS 连接。
///
/// 默认**不走代理**：XMTV 是国内的 CDN，把几百兆的视频塞进本机代理只会更慢
/// （实测代理繁忙时下载速度从 86 MB/min 掉到 6 MB/min）。
/// 确实需要走代理时设 `XMTV_USE_PROXY=1`。
static CLIENT: Lazy<Client> = Lazy::new(|| {
    let builder = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36",
        )
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90));

    let use_proxy = std::env::var("XMTV_USE_PROXY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let builder = if use_proxy {
        info!("XMTV_USE_PROXY 已开启，请求将走系统代理");
        builder
    } else {
        // reqwest 默认会读 HTTP_PROXY/HTTPS_PROXY 环境变量，这里明确禁用
        builder.no_proxy()
    };

    builder.build().expect("构建 reqwest Client 失败")
});

/// 供外部复用同一个连接池（例如下载视频）。
pub fn client() -> &'static Client {
    &CLIENT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoUrl {
    pub title: String,
    pub name: String,
    pub url: String,
    pub time: u128,
    /// XMTV 侧的稳定唯一 id，用来做去重键（旧的 json 缺这个字段时为 0）
    #[serde(default)]
    pub id: u64,
}

impl VideoUrl {
    /// 把 `name` 转成可以直接当文件名用的字符串：
    /// 去掉 Windows/Unix 下的非法字符，并限制长度，避免下载时创建文件失败。
    pub fn safe_file_stem(&self) -> String {
        sanitize_file_stem(&self.name)
    }
}

/// 去掉文件名中的非法字符并限制长度（按字符而不是字节截断，避免切坏 UTF-8）。
pub fn sanitize_file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim();
    // Windows 单个路径分量上限 255，这里留足余量给扩展名和临时后缀
    let truncated: String = cleaned.chars().take(120).collect();
    if truncated.is_empty() {
        "video".to_string()
    } else {
        truncated
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub title: String,
    pub range: Vec<VideoUrl>,
}

impl PartialEq for VideoUrl {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title && self.name == other.name && self.time == other.time
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexPicture {
    host: String,
    dir: String,
    path: String,
    filepath: String,
    filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentUrls {
    pub www: String,
    pub h5: String,
    pub share: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoExtra {
    pub site_name: String,
    pub is_top: u8,
    pub is_hot: u8,
    pub is_slide: u8,
    pub is_headline: u8,
    pub status: u64,
    pub label_ids: Vec<u64>,
    pub order_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: u64,
    pub site_id: u64,
    pub module_id: String,
    pub bundle_id: String,
    pub r#type: String,
    pub title: String,
    pub content_id: u64,
    pub content_from_id: u64,
    pub detail_id: u64,
    pub create_time: u128,
    pub indexpic: IndexPicture,
    pub publish_time: u128,
    pub is_publish: u8,
    pub column_id: u64,
    pub main_column: u64,
    pub parents_column: Vec<String>,
    pub content_urls: ContentUrls,
    pub extra: VideoExtra,
    pub brief: Option<String>,
    pub source: String,
    pub outlink: String,
    pub publish_time_stamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchBody {
    pub total: usize,
    pub data: Vec<VideoInfo>,
}

const SEARCH_URL: &str = "https://mapi1.kxm.xmtv.cn/api/open/xiamen/web_search_list.php?count=10000&search_text=%E6%96%97%E9%98%B5%E6%9D%A5%E7%9C%8B%E6%88%8F&offset=0&bundle_id=livmedia&order_by=publish_time&time=0&with_count=1";

/// 带指数退避的重试；只在真正失败时返回 Err，避免过去那种死循环空转。
async fn retry<T, F, Fut>(what: &str, attempts: usize, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last: Option<anyhow::Error> = None;
    for i in 0..attempts {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                warn!("{what} 第 {} 次失败: {e}", i + 1);
                last = Some(e);
                if i + 1 < attempts {
                    // 上游会限流，退避要给得够久，否则五次重试十几秒就用完了
                    tokio::time::sleep(Duration::from_secs(5 * 2u64.pow(i.min(4) as u32))).await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("{what} 失败")))
}

pub async fn get_search_body() -> Result<SearchBody> {
    let url = Url::parse(SEARCH_URL)?;
    info!("获取视频列表 url = {:?}", url);

    let body = retry("获取视频列表", 5, || {
        let url = url.clone();
        async move {
            let res = CLIENT.get(url).send().await?.error_for_status()?;
            Ok(res.json::<SearchBody>().await?)
        }
    })
    .await?;

    // 过去这里是 assert!，接口多返回一条就会 panic 掉整个流程，改成告警。
    if body.data.len() != body.total {
        warn!(
            "接口返回条数 {} 与 total {} 不一致，按实际返回的条数继续",
            body.data.len(),
            body.total
        );
    }
    Ok(body)
}

pub async fn get() -> Result<Vec<VideoUrl>> {
    let body = get_search_body().await?;

    let mut ret: Vec<VideoUrl> = Vec::new();
    let data = body.data;
    info!("获取到视频列表 共 {} 条", data.len());

    for ele in data {
        match to_video_url(&ele) {
            Ok(video) => {
                info!("获取到单个视频信息 video = {:?}", video);
                ret.push(video);
            }
            Err(why) => {
                // 单条解析不了**只跳过这一条**。0.2.2 就是栽在这里：
                // 某条切不出日期就让整次更新失败，上游新增一个别的栏目
                // 就能把所有人的更新全部打掉。
                warn!("{why}，已跳过这一条");
                warn!("  title = {:?}", ele.title);
                warn!("  share = {:?}", ele.content_urls.share);
            }
        }
    }
    Ok(ret)
}

/// 把接口返回的一条记录转成 [`VideoUrl`]。解析不出来返回 `Err`，调用方跳过它。
///
/// 抽成一个不联网的纯函数是有意的：上游接口一改，这里就是第一个出问题的地方，
/// 而拿真实报文喂它就能在测试里提前发现（见 `tests/real_payload.rs`）。
pub fn to_video_url(info: &VideoInfo) -> Result<VideoUrl, &'static str> {
    let title = extract_play_title(&info.title).ok_or("解析不出剧目名")?;
    // 先从标题里找日期，找不到再退回分享链接。
    // 过去是按空格切开取第 1 段，标题里多打一个空格（上游确实有这种数据）
    // 就会切出空串，整条被丢掉。
    let time = extract_date(&info.title)
        .or_else(|| extract_date(&info.content_urls.share))
        .ok_or("解析不出播出日期")?;
    Ok(VideoUrl {
        title,
        name: info.title.clone(),
        url: info.content_urls.share.clone(),
        time,
        id: info.id,
    })
}

/// 从条目标题里解析出剧目名。标题的格式是 `{剧目名}（N） 斗阵来看戏 {日期} - 厦门卫视`。
///
/// 关键是要用 `rfind` 而不是 `find`：有的标题里「斗阵来看戏」会出现两次，
/// 例如 `《斗阵来看戏》栏目曾小真歌仔戏剧团签约仪式 斗阵来看戏 2026.05.28 - 厦门卫视`，
/// 用 `find` 会命中 `《》` 里面那个，剧目名被切成单个 `《`，
/// 结果就是投出一个标题叫「《」的垃圾稿件。
pub fn extract_play_title(name: &str) -> Option<String> {
    const KEYWORD: &str = "斗阵来看戏";
    let head = match name.rfind(KEYWORD) {
        Some(p) => &name[..p],
        None => name,
    };
    let title = head
        .replace('（', "(")
        .split('(')
        .next()
        .unwrap_or_default()
        .replace(' ', "");
    let title = title.trim().to_string();

    // 只剩标点/括号的不是剧目名
    if title.is_empty()
        || !title
            .chars()
            .any(|c| c.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&c))
    {
        return None;
    }
    Some(title)
}

/// 从一段文本里找出第一个形如 `2026.08.17` / `2026-08-17` / `20260817` 的日期，
/// 返回 `20260817` 这样的数字。
///
/// 不再依赖"日期一定在第几个空格之后"这种脆弱假设：上游标题里空格数量并不稳定，
/// 按位置切分会把整条数据丢掉。
pub fn extract_date(text: &str) -> Option<u128> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        // 前一个字符也是数字的话，说明我们在一串数字的中间，跳过
        if start > 0 && bytes[start - 1].is_ascii_digit() {
            continue;
        }
        let mut digits = String::with_capacity(8);
        for &b in &bytes[start..] {
            if b.is_ascii_digit() {
                digits.push(b as char);
                if digits.len() == 8 {
                    break;
                }
            } else if (b == b'.' || b == b'-') && !digits.is_empty() {
                // 允许 2026.08.17 这样的分隔符
                continue;
            } else {
                break;
            }
        }
        if digits.len() != 8 {
            continue;
        }
        // 粗略校验年月日，避免把别的数字串当成日期
        let year: u32 = digits[0..4].parse().ok()?;
        let month: u32 = digits[4..6].parse().ok()?;
        let day: u32 = digits[6..8].parse().ok()?;
        if (1900..=2999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day) {
            return digits.parse().ok();
        }
    }
    None
}

/// 从分享页面里抠出真正的 mp4 地址。
///
/// 过去用 `find(..).unwrap_or(0) + 13` 做切片，页面结构变化时会静默返回一段垃圾字符串，
/// 下载阶段才以奇怪的方式失败。现在找不到就直接报错。
pub async fn get_video_url(url: &str) -> Result<String> {
    let parsed = Url::parse(url).with_context(|| format!("解析分享链接失败 url = {url}"))?;
    info!("获取视频页面 url = {url:?}");

    let download_url = retry("解析视频源地址", 5, || {
        let parsed = parsed.clone();
        async move {
            let text = CLIENT
                .get(parsed)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            extract_source_url(&text)
        }
    })
    .await
    .with_context(|| format!("无法从 {url} 解析出视频源地址"))?;

    info!("从 {url:?} 获取到视频源地址 {download_url:?}");
    Ok(download_url)
}

/// 从 HTML 里解析 `<source src="...">`，纯函数，方便单测。
pub fn extract_source_url(html: &str) -> Result<String> {
    const TAG: &str = "<source src=";
    let start = html
        .find(TAG)
        .ok_or_else(|| anyhow!("页面中没有找到 `{TAG}`，站点结构可能已变化"))?
        + TAG.len();
    let rest = &html[start..];
    // src 后面可能是 " 也可能是 '
    let quote = rest
        .chars()
        .next()
        .ok_or_else(|| anyhow!("`{TAG}` 之后没有内容"))?;
    let rest = if quote == '"' || quote == '\'' {
        &rest[quote.len_utf8()..]
    } else {
        rest
    };
    let end = rest
        .find(['"', '\''])
        .ok_or_else(|| anyhow!("视频地址没有闭合引号"))?;
    let download_url = rest[..end].trim().to_string();
    if download_url.is_empty() {
        return Err(anyhow!("解析出的视频地址为空"));
    }
    Ok(download_url)
}

pub fn sort_by_title(urls: Vec<VideoUrl>) -> Vec<Video> {
    let mut videos: Vec<Video> = Vec::new();
    for url in urls {
        // 过去这里遍历完所有 video 都不 break，同名的会被塞进每一个分组里。
        match videos.iter_mut().find(|v| v.title == url.title) {
            Some(video) => video.range.push(url),
            None => videos.push(Video {
                title: url.title.clone(),
                range: vec![url],
            }),
        }
    }
    for video in &mut videos {
        video.range.sort_by_key(|a| a.time);
        // 同一个 id 只保留一条，防止上游列表里出现重复条目
        video.range.dedup_by(|a, b| a.id != 0 && a.id == b.id);
    }
    videos
}

pub fn resort(videos: Vec<Video>) -> Vec<VideoUrl> {
    let mut urls = Vec::new();
    for video in videos {
        urls.extend(video.range);
    }
    urls
}

#[derive(Debug)]
pub struct Videoplay {
    pub name: String,
    pub url: String,
}

pub async fn get_video_to_url(mut videos: Vec<VideoUrl>) -> Result<Vec<VideoUrl>> {
    let len = videos.len().try_into()?;
    let pb = ProgressBar::new(len);
    pb.set_style(ProgressStyle::default_bar()
    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({per_sec}, {eta})")?);
    for video in &mut videos {
        if video.url.ends_with(".mp4") {
            warn!("检测到已获得地址");
        } else {
            match get_video_url(&video.url).await {
                Ok(ret) => video.url = ret,
                // 单条失败不再无限重试（get_video_url 内部已经重试过了），跳过即可
                Err(e) => error!("获取源url失败 video = {:?} err = {e}", video),
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("源url获取完成");
    Ok(videos)
}

pub fn get_random_url_list(videos: &[Video]) -> Result<Vec<Videoplay>> {
    if videos.is_empty() {
        return Err(anyhow!("视频列表为空，无法随机挑选"));
    }
    let mut rng = rand::thread_rng();
    let randnumber = rng.gen_range(0..videos.len());
    let randone = &videos[randnumber];
    let ret = randone
        .range
        .iter()
        .map(|i| Videoplay {
            name: i.name.clone(),
            url: i.url.clone(),
        })
        .collect();
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_source_url() {
        let html = r#"<video><source src="https://example.com/a.mp4" type="video/mp4"></video>"#;
        assert_eq!(
            extract_source_url(html).unwrap(),
            "https://example.com/a.mp4"
        );
        let html_single = r#"<source src='https://example.com/b.mp4'>"#;
        assert_eq!(
            extract_source_url(html_single).unwrap(),
            "https://example.com/b.mp4"
        );
        assert!(extract_source_url("<html>没有视频</html>").is_err());
    }

    #[test]
    fn test_extract_play_title() {
        assert_eq!(
            extract_play_title("皇家奇缘（1） 斗阵来看戏 2026.08.17 - 厦门卫视").as_deref(),
            Some("皇家奇缘")
        );
        assert_eq!(
            extract_play_title("三娘教子 斗阵来看戏 2026.04.27 - 厦门卫视").as_deref(),
            Some("三娘教子")
        );
        // 「斗阵来看戏」出现两次：用 find 会把剧目名切成单个「《」，必须用 rfind
        assert_eq!(
            extract_play_title(
                "《斗阵来看戏》栏目曾小真歌仔戏剧团签约仪式 斗阵来看戏 2026.05.28 - 厦门卫视"
            )
            .as_deref(),
            Some("《斗阵来看戏》栏目曾小真歌仔戏剧团签约仪式")
        );
        // 切出来只剩标点的，不是剧目名
        assert_eq!(extract_play_title("《 斗阵来看戏 2026.05.28"), None);
        assert_eq!(extract_play_title("斗阵来看戏 2026.05.28"), None);
    }

    #[test]
    fn test_extract_date() {
        assert_eq!(
            extract_date("皇家奇缘（1） 斗阵来看戏 2026.08.17 - 厦门卫视"),
            Some(20260817)
        );
        // 上游确实有标题里多一个空格的数据，过去这种会被整条丢掉
        assert_eq!(
            extract_date("三代奇缘（1） 斗阵来看戏  2025.12.22 - 厦门卫视"),
            Some(20251222)
        );
        // 标题里 《斗阵来看戏》 出现两次，也不该影响取日期
        assert_eq!(
            extract_date("《斗阵来看戏》栏目签约仪式 斗阵来看戏 2026.05.28 - 厦门卫视"),
            Some(20260528)
        );
        assert_eq!(
            extract_date("莫愁女（3） 斗阵来看戏 2025-12-19"),
            Some(20251219)
        );
        assert_eq!(extract_date("紧凑格式 20240131 结尾"), Some(20240131));
        // 集数那种一两位数字不能被当成日期
        assert_eq!(extract_date("白蛇传（3） 斗阵来看戏"), None);
        // 月份 13 不合法
        assert_eq!(extract_date("20261301"), None);
    }

    #[test]
    fn test_sanitize_file_stem() {
        assert_eq!(sanitize_file_stem("a/b:c?d"), "a_b_c_d");
        assert_eq!(sanitize_file_stem("   "), "video");
        assert!(sanitize_file_stem(&"字".repeat(500)).chars().count() <= 120);
    }

    #[test]
    fn test_sort_by_title_no_fanout() {
        let mk = |title: &str, id: u64, time: u128| VideoUrl {
            title: title.into(),
            name: format!("{title}-{id}"),
            url: String::new(),
            time,
            id,
        };
        let got = sort_by_title(vec![mk("甲", 1, 2), mk("甲", 2, 1), mk("乙", 3, 3)]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].range.len(), 2);
        // 按时间排序
        assert_eq!(got[0].range[0].id, 2);
        assert_eq!(got[1].range.len(), 1);
    }
}
