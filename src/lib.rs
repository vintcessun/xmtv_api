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
        let videos = video::get().await?;
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
        let file = std::fs::File::open(filename)?;
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
        let videos = video::get().await?;
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
    use reqwest::blocking::Client;
    use video::SearchBody;

    #[test]
    fn test_struct() {
        let res = Client::new().get("https://mapi1.kxm.xmtv.cn/api/open/xiamen/web_search_list.php?count=10000&search_text=%E6%96%97%E9%98%B5%E6%9D%A5%E7%9C%8B%E6%88%8F&offset=0&bundle_id=livmedia&order_by=publish_time&time=0&with_count=1").send().unwrap();
        let data = res.json::<SearchBody>().unwrap();
        println!("{} {}", data.data.len(), data.total)
    }
}
