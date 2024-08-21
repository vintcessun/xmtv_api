mod get_video_list;
pub mod sql;
use anyhow::Result;
pub use get_video_list::*;

#[derive(Debug)]
pub struct Videos {
    pub videos: Vec<Video>,
}

impl Videos {
    pub fn get() -> Result<Self> {
        let ret = sql::get()?;
        let videos = get_video_list::resort(ret);
        Ok(Self { videos })
    }

    pub fn random(&self) -> Result<Vec<Videoplay>> {
        get_video_list::get_random_url_list(&self.videos)
    }

    pub fn index(&self, index: usize) -> Video {
        self.videos[index].clone()
    }

    pub fn renew(&mut self) -> Result<()> {
        let ret = sql::get()?;
        let videos = get_video_list::resort(ret);
        *self = Self { videos };
        Ok(())
    }
}
