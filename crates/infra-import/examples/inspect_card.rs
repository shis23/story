use storyforge_infra_import::import_character;
use std::fs;

fn main() {
    let data = fs::read("test-card.png").expect("读取失败");
    let card = import_character(&data).expect("解析失败");

    println!("=== 基本信息 ===");
    println!("name: {}", card.name);
    println!("description: {}", truncate(&card.description, 150));
    println!("personality: {}", truncate(&card.personality, 100));
    println!("scenario: {}", truncate(&card.scenario, 100));
    println!("first_mes (前300字): {}", truncate(&card.first_mes, 300));
    println!("system_prompt: {}", truncate(&card.system_prompt, 100));
    println!("tags: {:?}", card.tags);
    println!("creator: {}", card.creator);
    println!("spec_version: {}", card.spec_version);
    println!("alternate_greetings: {}", card.alternate_greetings.len());
    for (i, g) in card.alternate_greetings.iter().enumerate() {
        println!("  greeting[{}]: {}", i, truncate(g, 80));
    }

    println!("\n=== 世界书 ===");
    if let Some(book) = &card.embedded_world_info {
        println!("总条目: {}", book.entries.len());
        let constants: Vec<_> = book.entries.iter().filter(|e| e.constant).collect();
        let selectives: Vec<_> = book.entries.iter().filter(|e| e.selective).collect();
        println!("蓝灯(constant): {}", constants.len());
        println!("绿灯(selective): {}", selectives.len());
        println!("\n--- 蓝灯条目（前10条）---");
        for (i, e) in constants.iter().take(10).enumerate() {
            println!("  [{}] keys={:?} content={}", i, e.keys, truncate(&e.content, 120));
        }
        println!("\n--- 绿灯条目（前10条）---");
        for (i, e) in selectives.iter().take(10).enumerate() {
            println!("  [{}] keys={:?} content={}", i, e.keys, truncate(&e.content, 120));
        }
    } else {
        println!("无内嵌世界书");
    }

    println!("\n=== Extensions ===");
    let ext = &card.extensions;
    if let Some(obj) = ext.as_object() {
        println!("顶层 keys: {:?}", obj.keys().collect::<Vec<_>>());
        for key in obj.keys() {
            let v = &obj[key];
            let preview = truncate(&serde_json::to_string(v).unwrap_or_default(), 150);
            println!("  {}: {}", key, preview);
        }
    } else {
        println!("extensions 不是 object: {}", truncate(&serde_json::to_string(ext).unwrap_or_default(), 200));
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}
