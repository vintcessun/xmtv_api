//! 拿**真实抓下来的接口报文**跑一遍解析，不联网。
//!
//! 为什么要有这个文件：0.2.2 出过一次事故 —— 上游标题里没有「斗阵来看戏」时，
//! 解析代码硬从第 15 个字节开始切，切出一段中文拿去 `parse()`，
//! 报 `invalid digit found in string`，于是**整次更新全部失败**。
//! 那种问题单元测试测不出来，因为手写的用例永远只覆盖你想得到的形状。
//!
//! `fixtures/` 里是 `curl` 从 `mapi1.kxm.xmtv.cn` 原样存下来的响应：
//!
//! * `search_20.json` —— 最新的 20 条（2026 年，标题带空格和全角括号）
//! * `search_oldest.json` —— 档案末尾的 11 条（2020 年，格式明显不一样：
//!   括号后面没有空格，还有整条没有集数的）
//!
//! 这两份一起，覆盖了上游六年里的格式变化。

use xmtv_api::{SearchBody, extract_date, extract_play_title, sort_by_title, to_video_url};

const 最新: &str = include_str!("fixtures/search_20.json");
const 最老: &str = include_str!("fixtures/search_oldest.json");

fn 解析(body: &str) -> SearchBody {
    serde_json::from_str(body).expect("结构体和上游返回的 JSON 对不上了 —— 接口八成改了")
}

/// 最基本的一条：结构体还能不能装下上游返回的东西。
///
/// 接口加字段不会让它失败（serde 默认忽略多余字段），但**删字段或改类型会**，
/// 而那正是最需要第一时间知道的事。
#[test]
fn 上游报文还能被结构体装下() {
    let 新 = 解析(最新);
    let 老 = 解析(最老);
    assert_eq!(新.data.len(), 20);
    // 这一份是从档案末尾（offset=2271）抓的，上游只给了 11 条就没了 ——
    // 顺带说明一件事：`data.len()` 和 `total` 本来就可以对不上，
    // 0.2.2 在这里写的是 `assert!`，接口一少返回就 panic 掉整个流程
    assert_eq!(老.data.len(), 11);
    assert!(老.data.len() < 老.total, "末尾那一页本来就该比 total 少");
    assert!(新.total > 2000, "total 看起来不对: {}", 新.total);
}

/// 核心回归：真实数据里**每一条**都要能解析出来。
///
/// 这就是 0.2.2 那次事故的直接防线。哪怕只有一条解析不了，也说明解析规则
/// 已经跟不上上游的数据了，应该立刻知道，而不是等用户报「更新失败」。
#[test]
fn 真实数据里没有一条解析不了() {
    for (来源, body) in [("最新 20 条", 最新), ("最老 20 条", 最老)] {
        let parsed = 解析(body);
        let mut 失败 = Vec::new();
        for info in &parsed.data {
            if let Err(why) = to_video_url(info) {
                失败.push(format!("  {why}: {}", info.title));
            }
        }
        assert!(
            失败.is_empty(),
            "{来源} 里有 {} 条解析不了：\n{}",
            失败.len(),
            失败.join("\n")
        );
    }
}

/// 2020 年那批的格式和现在不一样，单独盯一下。
///
/// 那时候的标题是 `碧海青天（3）斗阵来看戏 2020.03.02 - 厦门卫视`：
/// 全角括号后面**没有空格**，而且有整条连集数都没有的
/// （`碧海青天 斗阵来看戏 2020.02.29`）。按空格切分的老写法在这两种上都会翻车。
#[test]
fn 六年前的老格式也认得() {
    let parsed = 解析(最老);
    let videos: Vec<_> = parsed
        .data
        .iter()
        .filter_map(|i| to_video_url(i).ok())
        .collect();

    let 碧海青天: Vec<_> = videos.iter().filter(|v| v.title == "碧海青天").collect();
    assert!(
        碧海青天.len() >= 3,
        "碧海青天应该有好几集，实际 {}",
        碧海青天.len()
    );

    // 没有集数括号的那条，剧目名不能带上后面的栏目名
    let 无集数 = videos
        .iter()
        .find(|v| v.name.contains("碧海青天 斗阵来看戏"))
        .expect("样本里应该有一条没有集数的");
    assert_eq!(无集数.title, "碧海青天");

    // 日期要从标题里取到，而不是退回去用分享链接里的日期
    // （这两者对不上：分享页的日期是补录日期，不是播出日期）
    assert!(
        videos.iter().all(|v| v.time / 10000 == 2020),
        "2020 年那批的日期应该都是 2020 年"
    );
}

/// 最新那批的具体取值，钉死几个已知答案。
#[test]
fn 最新一批的解析结果对得上() {
    let parsed = 解析(最新);
    let first = to_video_url(&parsed.data[0]).expect("第一条应该能解析");

    assert_eq!(first.title, "大名春秋");
    assert_eq!(first.time, 20260902);
    assert_eq!(first.id, 670419);
    assert!(
        first.url.starts_with("https://share1.kxm.xmtv.cn/xmtv/"),
        "分享地址形状变了: {}",
        first.url
    );
}

/// 同一部戏要被归到一组，而且按播出时间排好。
#[test]
fn 分组和排序() {
    let parsed = 解析(最新);
    let videos: Vec<_> = parsed
        .data
        .iter()
        .filter_map(|i| to_video_url(i).ok())
        .collect();
    let 总数 = videos.len();
    let grouped = sort_by_title(videos);

    // 分组不能凭空多出或少掉条目
    let 分组后总数: usize = grouped.iter().map(|g| g.range.len()).sum();
    assert_eq!(分组后总数, 总数, "分组前后条数对不上");

    // 组内按时间从早到晚
    for g in &grouped {
        let times: Vec<_> = g.range.iter().map(|v| v.time).collect();
        let mut sorted = times.clone();
        sorted.sort_unstable();
        assert_eq!(times, sorted, "《{}》组内没有按时间排序", g.title);
    }

    // 剧目名不能重复出现在多个组里（0.2.2 之前的 sort_by_title 会把同名的
    // 塞进每一个已有分组）
    let mut 名字: Vec<_> = grouped.iter().map(|g| g.title.clone()).collect();
    let 分组数 = 名字.len();
    名字.sort();
    名字.dedup();
    assert_eq!(名字.len(), 分组数, "有剧目名出现在多个分组里");
}

/// 标题里带《斗阵来看戏》的那种，剧目名不能被切成一个「《」。
///
/// 这条在单元测试里已经有了，这里再用真实数据的形状确认一遍：
/// 上游确实存在这样的条目，而且用 `find` 的老写法会投出标题叫「《」的稿件。
#[test]
fn 栏目名出现两次时不会切坏() {
    let name = "《斗阵来看戏》栏目曾小真歌仔戏剧团签约仪式 斗阵来看戏 2026.05.28 - 厦门卫视";
    assert_eq!(
        extract_play_title(name).as_deref(),
        Some("《斗阵来看戏》栏目曾小真歌仔戏剧团签约仪式")
    );
    assert_eq!(extract_date(name), Some(20260528));
}
