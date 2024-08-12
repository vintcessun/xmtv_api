mod sql;
mod get_video_list;
use get_video_list::{Video, Videoplay};
use anyhow::Result;

#[derive(Debug)]
pub struct Videos{
    videos: Vec<Video>
}

impl Videos{
    pub fn get(&mut self)->Result<Self>{
        let ret = sql::get()?;
        let videos = get_video_list::resort(ret);
        Ok(Self{videos})
    }

    pub fn random(&self)->Result<Vec<Videoplay>>{
        get_video_list::get_random_url_list(&self.videos)
    }

    pub fn index(&self, index: usize)->Video{
        self.videos[index].clone()
    }
}



