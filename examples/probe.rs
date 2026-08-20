//! 联网自检：拉列表 -> 解析真实 mp4 地址 -> 下载一小段 -> 校验。
//!
//! `cargo run --example probe`

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let urls = xmtv_api::get().await?;
    println!("片源条目数: {}", urls.len());
    let groups = xmtv_api::sort_by_title(urls.clone());
    println!("剧目数: {}", groups.len());

    let multi: Vec<_> = groups.iter().filter(|g| g.range.len() >= 2).collect();
    println!("有 >=2 个分P 的剧目数: {}", multi.len());
    let sample = multi.first().expect("至少要有一部多集的戏");
    println!("样例剧目: {} 共 {} 集", sample.title, sample.range.len());
    for p in &sample.range {
        println!("   id={} time={} name={}", p.id, p.time, p.name);
        println!("      文件名 -> {}.mp4", p.safe_file_stem());
    }

    let first = &sample.range[0];
    let src = xmtv_api::get_video_url(&first.url).await?;
    println!("解析到源地址: {src}");
    assert!(src.starts_with("http"), "源地址必须是 http(s)");

    // 只拉前 2MB，验证链路真的能下下来
    let res = xmtv_api::client()
        .get(&src)
        .header("Range", "bytes=0-2097151")
        .send()
        .await?
        .error_for_status()?;
    println!(
        "HTTP {} content-length={:?}",
        res.status(),
        res.content_length()
    );
    let bytes = res.bytes().await?;
    println!("实际下载 {} 字节", bytes.len());
    assert!(bytes.len() > 100_000, "下载内容太小，链路有问题");
    // mp4 的 ftyp box 在文件开头
    println!("文件头: {:?}", &bytes[..16.min(bytes.len())]);
    assert_eq!(&bytes[4..8], b"ftyp", "下载到的不是 mp4");
    println!("\n全部通过：XMTV 列表 / 源地址解析 / 实际下载 都正常");
    Ok(())
}
