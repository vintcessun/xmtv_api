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
static CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36",
        )
        .connect_timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .expect("构建 reqwest Client 失败")
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
                    tokio::time::sleep(Duration::from_secs(2u64.pow(i.min(4) as u32))).await;
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
        let id = ele.id;
        let name = ele.title;
        let position = name.find("斗阵来看戏").unwrap_or(name.len());
        let title = name[0..position]
            .replace('（', "(")
            .split('(')
            .collect::<Vec<_>>()[0]
            .replace(' ', "");
        let url_into_share = ele.content_urls.share;
        let position = name.find("斗阵来看戏").unwrap_or(0) + "斗阵来看戏".len();
        let t: &str = &name[position..];
        let t = t.split(' ').collect::<Vec<_>>();
        let t = if t.len() >= 2 {
            t[1].replace(['.', '-'], "")
        } else {
            match url_into_share.find('-') {
                Some(_) => {
                    let parts = url_into_share.split('/').collect::<Vec<_>>();
                    match parts.get(4) {
                        Some(t) => t.replace(['.', '-'], ""),
                        None => {
                            warn!(
                                "无法从分享链接推断日期，已忽略 name = {name:?} url = {url_into_share:?}"
                            );
                            continue;
                        }
                    }
                }
                _ => {
                    error!("存在一些无法识别的组别已经忽略，下面是一些信息或许有助于修复");
                    warn!("title = {:?}", title);
                    warn!("name = {:?}", name);
                    warn!("url_into_share = {:?}", url_into_share);
                    continue;
                }
            }
        };
        // 过去这里用 `?`，一条脏数据会让整次抓取失败，现在只跳过这一条。
        let t: u128 = match t.parse() {
            Ok(t) => t,
            Err(e) => {
                warn!("解析日期 {t:?} 失败({e})，已忽略 name = {name:?}");
                continue;
            }
        };
        let video = VideoUrl {
            title,
            name,
            url: url_into_share,
            time: t,
            id,
        };
        info!("获取到单个视频信息 video = {:?}", video);
        ret.push(video);
    }
    Ok(ret)
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
