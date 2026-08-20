mod video;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;
pub use video::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Videos {
    pub videos: Vec<Video>,
    pub last_update: i64,
}

impl Videos {
    pub async fn get_from_internet() -> Result<Self> {
        let ts = Utc::now().timestamp();
        let urls = video::get().await?;
        let videos = video::sort_by_title(urls);
        Ok(Self {
            videos,
            last_update: ts,
        })
    }

    pub async fn read_from_file_tokio<P: AsRef<Path>>(filename: P) -> Result<Self> {
        let file = tokio::fs::read_to_string(filename).await?;
        Ok(serde_json::from_str::<Self>(&file)?)
    }

    pub fn read_from_file<P: AsRef<Path>>(filename: P) -> Result<Self> {
        let file = std::fs::File::open(filename)?;
        let reader = std::io::BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }
}

impl Videos {
    pub async fn save_to_file_tokio<P: AsRef<Path>>(&self, filename: P) -> Result<()> {
        let json = serde_json::to_string(self)?;
        tokio::fs::write(filename, json.as_bytes()).await?;
        Ok(())
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, filename: P) -> Result<()> {
        // 这里原本是 File::open，只读句柄写不进去，保存永远失败
        let file = std::fs::File::create(filename)?;
        let writer = std::io::BufWriter::new(file);
        serde_json::to_writer(writer, self)?;
        Ok(())
    }
}

impl Videos {
    pub fn random(&self) -> Result<Vec<Videoplay>> {
        video::get_random_url_list(&self.videos)
    }

    pub fn index(&self, index: usize) -> Video {
        self.videos[index].clone()
    }
}

impl Videos {
    pub async fn renew(&mut self) -> Result<()> {
        let last_ts = self.last_update;
        let ts = Utc::now().timestamp();
        if ts - last_ts <= 24 * 60 * 60 {
            return Ok(());
        }
        let urls = video::get().await?;
        let videos = video::sort_by_title(urls);
        *self = Self {
            videos,
            last_update: ts,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 需要联网，验证上游接口返回的 JSON 仍然能被结构体解析。
    #[tokio::test]
    #[ignore = "需要联网"]
    async fn test_struct() {
        let data = video::get_search_body().await.unwrap();
        println!("{} {}", data.data.len(), data.total)
    }

    #[test]
    fn test_save_and_read_roundtrip() {
        let dir = std::env::temp_dir().join("xmtv_api_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("videos.json");
        let videos = Videos {
            videos: vec![video::Video {
                title: "测试".into(),
                range: vec![],
            }],
            last_update: 42,
        };
        videos.save_to_file(&path).unwrap();
        let back = Videos::read_from_file(&path).unwrap();
        assert_eq!(back.last_update, 42);
        assert_eq!(back.videos.len(), 1);
        std::fs::remove_file(&path).ok();
    }
}
