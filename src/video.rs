use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use log::{error, info, warn};
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoUrl {
    pub title: String,
    pub name: String,
    pub url: String,
    pub time: u128,
}

impl PartialEq for VideoUrl {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title && self.name == other.name && self.time == other.time
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub title: String,
    pub range: Vec<VideoUrl>,
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

pub async fn get_search_body() -> Result<SearchBody> {
    let url = Url::parse("https://mapi1.kxm.xmtv.cn/api/open/xiamen/web_search_list.php?count=10000&search_text=%E6%96%97%E9%98%B5%E6%9D%A5%E7%9C%8B%E6%88%8F&offset=0&bundle_id=livmedia&order_by=publish_time&time=0&with_count=1")?;
    info!("获取视频列表 url = {:?}", &url);

    let res = Client::new().get(url).send().await?;
    let body = res.json::<SearchBody>().await?;
    assert!(body.data.len() == body.total, "解析数量必须和请求数量相同");
    Ok(body)
}

pub async fn get() -> Result<Vec<VideoUrl>> {
    let body = get_search_body().await?;

    let mut ret: Vec<VideoUrl> = Vec::new();
    let data = body.data;
    info!("获取到视频列表 data = {:?}", &data);

    for ele in data {
        let name = ele.title;
        let position = match name.find("斗阵来看戏") {
            Some(ret) => ret,
            _ => name.len(),
        };
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
                    let t = url_into_share.split('/').collect::<Vec<_>>();
                    let t = t[4];
                    t.replace(['.', '-'], "")
                }
                _ => {
                    error!("存在一些无法识别的组别已经忽略，下面是一些信息或许有助于修复");
                    warn!("titile = {:?}", &title);
                    warn!("name = {:?}", &name);
                    warn!("url_into_share = {:?}", &url_into_share);
                    continue;
                }
            }
        };
        let t = t.parse()?;
        let video = VideoUrl {
            title,
            name,
            url: url_into_share,
            time: t,
        };
        info!("获取到单个视频信息 video = {:?}", &video);
        ret.push(video);
    }
    Ok(ret)
}

pub async fn get_video_url(url: &str) -> Result<String> {
    let url_into_share = Url::parse(url)?;
    info!("获取视频页面 url = {url:?}");

    let res = Client::new().get(url_into_share.clone()).send().await?;
    let text = res.text().await?;
    let text = text[(text.find("<source src=").unwrap_or(0) + 13)..].to_string();
    let download_url = text[..(text.find('\"').unwrap_or(0))].to_string();
    info!("从 {:?} 获取到视频源地址 {:?}", &url, &download_url);
    Ok(download_url)
}

pub fn sort_by_title(urls: Vec<VideoUrl>) -> Vec<Video> {
    let mut videos: Vec<Video> = Vec::new();
    for url in urls {
        let mut exists = false;
        for video in &mut videos {
            if url.title == video.title {
                exists = true;
                video.range.push(url.clone());
            }
        }
        if !exists {
            let mut video = Video {
                title: url.title.clone(),
                range: Vec::new(),
            };
            video.range.push(url.clone());
            videos.push(video);
        }
    }
    for video in &mut videos {
        video.range.sort_by(|a, b| a.time.cmp(&b.time));
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
        video.url = loop {
            if video.url.ends_with(".mp4") {
                warn!("检测到已获得地址");
                break video.url.clone();
            }
            match get_video_url(&video.url).await {
                Ok(ret) => {
                    warn!("成功获取 ret = {:?}", &ret);
                    break ret;
                }
                Err(_) => {
                    error!("获取源url失败 video = {:?}", &video);
                }
            }
        };
        pb.inc(1);
    }
    pb.finish_with_message("源url获取完成");
    Ok(videos)
}

pub fn get_random_url_list(videos: &[Video]) -> Result<Vec<Videoplay>> {
    let mut rng = rand::thread_rng();
    let randnumber = rng.gen_range(0..videos.len());
    let randone = &videos[randnumber];
    let mut ret = Vec::with_capacity(12);
    for i in &randone.range {
        let name = i.name.clone();
        let url = i.url.clone();
        let one = Videoplay { name, url };
        ret.push(one);
    }
    Ok(ret)
}
