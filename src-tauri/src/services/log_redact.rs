use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::utils::log::log;

/// 日志脱敏器：把文件名/路径替换为编号占位符，保留结构化技术信息
/// 同一个文件名/路径在整个会话中映射到同一编号，保持日志关联性
pub struct LogRedactor {
    map: HashMap<String, String>,
    counter: usize,
}

impl LogRedactor {
    pub fn new() -> Self {
        Self { map: HashMap::new(), counter: 0 }
    }

    fn placeholder_for(&mut self, raw: &str) -> String {
        if let Some(p) = self.map.get(raw) {
            return p.clone();
        }
        self.counter += 1;
        let p = format!("[N{}]", self.counter);
        self.map.insert(raw.to_string(), p.clone());
        p
    }

    /// 脱敏单行日志
    pub fn redact_line(&mut self, line: &str) -> String {
        // 匹配常见的 key=value 形式，value 可能含中文/特殊字符，以结束引号或行尾为准
        let mut out = line.to_string();
        for key in ["file=", "file_path=", "alist_path=", "target_path=", "target=", "folder_target=", "target_dir=", "out_dir=", "src=", "file_name=", "folder_name=", "task_name="] {
            out = redact_kv(&mut self.map, &mut self.counter, &out, key);
        }
        // 错误信息中的裸文件名兜底：中文连续段落（>=2 个连续中文字符的词组）替换
        out = redact_cjk_runs(&mut self.map, &mut self.counter, &out);
        out
    }
}

/// 对 `key=value` 模式脱敏：value 从 key= 后取到行尾或下一个 ", " 分隔
fn redact_kv(map: &mut HashMap<String, String>, counter: &mut usize, line: &str, key: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find(key) {
        // 避免匹配到已替换的占位符或 key 中间（如 file_path= 里含 file= 前缀碰撞：file_path 中的 "file_" 不带 =，无碰撞）
        let value_start = pos + key.len();
        // value 结束条件：行尾，或遇到 ", "（日志中 kv 之间分隔），或 "] "
        let value_part = &rest[value_start..];
        let end = value_part.find(", ").map(|i| value_start + i).unwrap_or(rest.len());
        let raw_value = &rest[value_start..end];

        // 空值或纯数字/纯技术值不脱敏
        if raw_value.is_empty() || raw_value.chars().all(|c| c.is_ascii_digit()) {
            result.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }

        result.push_str(&rest[..value_start]);
        let placeholder = {
            if let Some(p) = map.get(raw_value) {
                p.clone()
            } else {
                *counter += 1;
                let p = format!("[N{}]", counter);
                map.insert(raw_value.to_string(), p.clone());
                p
            }
        };
        result.push_str(&placeholder);
        rest = &rest[end..];
    }
    result.push_str(rest);
    result
}

/// 兜底：替换错误信息里的中文连续片段（长度 >= 6 个中文字符的连续段）
/// 过短的不动，避免误伤正常提示语（如"上传失败"）
fn redact_cjk_runs(map: &mut HashMap<String, String>, counter: &mut usize, line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut result = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if is_cjk(c) {
            let start = i;
            while i < chars.len() && (is_cjk(chars[i]) || chars[i] == '·' || chars[i] == '【' || chars[i] == '】' || chars[i] == '（' || chars[i] == '）' || chars[i] == '、' || (chars[i] == ' ' && i + 1 < chars.len() && is_cjk(chars[i+1]))) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            // 已经含占位符的行段（前面已替换）跳过；固定提示语白名单
            const WHITELIST: [&str; 10] = [
                "上传失败通知", "上传成功", "上传出错", "准备重试", "达到最大重试次数",
                "检测到上次异常退出", "等待当前文件上传完成后停止", "队列已停止，等待人工处理",
                "文件已在队列中", "已标记为失败",
            ];
            if WHITELIST.iter().any(|w| run.contains(w)) {
                result.push_str(&run);
            } else if run.chars().filter(|c| is_cjk(*c)).count() >= 6 {
                // 长中文段视为敏感内容，整体映射
                let p = if let Some(p) = map.get(&run) {
                    p.clone()
                } else {
                    *counter += 1;
                    let p = format!("[N{}]", counter);
                    map.insert(run.clone(), p.clone());
                    p
                };
                result.push_str(&p);
            } else {
                result.push_str(&run);
            }
        } else {
            result.push(c);
            i += 1;
        }
    }
    result
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF   // CJK 基本区
        | 0x3400..=0x4DBF // 扩展 A
        | 0x3000..=0x303F // CJK 标点（含【】（））
        | 0xFF00..=0xFFEF // 全角字符
    )
}

/// 生成脱敏版日志文件：读取 src 文件逐行脱敏，写入 dst，返回 dst 路径
pub fn generate_redacted_log(src: &Path, dst_dir: &Path) -> Option<PathBuf> {
    let content = match fs::read_to_string(src) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("脱敏日志：读取原文失败: {}, error={}", src.display(), e));
            return None;
        }
    };

    let mut redactor = LogRedactor::new();
    let mut out = String::with_capacity(content.len() + 1024);
    for line in content.lines() {
        out.push_str(&redactor.redact_line(line));
        out.push('\n');
    }

    if let Err(e) = fs::create_dir_all(dst_dir) {
        log(&format!("脱敏日志：创建临时目录失败: {}", e));
        return None;
    }
    let dst = dst_dir.join(src.file_name()?);
    if let Err(e) = fs::write(&dst, out) {
        log(&format!("脱敏日志：写入失败: {}, error={}", dst.display(), e));
        return None;
    }
    Some(dst)
}
