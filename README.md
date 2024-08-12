# 解析xmtv的一个库

内部实现了sql库的链接，按照日期划分，每日sql过期

## 接口

### get_video_list.rs

#### pub fn get()->Result<Vec<VideoUrl>> 获取具体视频地址（分享地址），如果错误会无限重试

#### pub fn get_video_url(url:&String)->Result<String> 从分享地址获取具体视频地址（每个）

#### pub fn resort (urls:Vec<VideoUrl>)->Vec<Video> 按照日期分类视频

#### pub fn get_video_to_url(mut videos:Vec<VideoUrl>)->Result<Vec<VideoUrl>> 一组视频获取

#### pub fn get_random_url_list(videos:&[Video])->Result<Vec<Videoplay>> 获取随机一组视频

其中struct的定义

pub struct Videoplay{

  pub name:String,

  pub url:String

}

pub struct Video{

  pub title:String,

  pub range:Vec<VideoUrl>

}

pub struct VideoUrl{

  pub title:String,

  pub name:String,

  pub url:String,

  pub time:u32

}

### sql.rs

默认保存位置const DB:&str = "urls.db";

pub fn update()->Result<()> 更新db

pub fn get_exact()->Result<Vec<VideoUrl>> 封装好的一键获取视频（具体mp4）库

pub fn get()->Result<Vec<VideoUrl>>获取视频地址

### lib.rs

impl Videos{

  pub fn get(&mut self)->Result<()>

  pub fn random(&self)->Result<Vec<Videoplay>>

  pub fn index(&self, index: usize)->Video

}
