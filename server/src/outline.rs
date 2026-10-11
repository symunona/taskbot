//! Outline of a text file: headings for markdown, definitions for code.
//! One line regex per file type, no parser. A miss only costs the model a
//! plain page read.

use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;

const MAX_ENTRIES: usize = 200;
const MAX_ENTRY_CHARS: usize = 120;

const KOTLIN_MODIFIERS: &str = "public|private|internal|protected|open|abstract|override|suspend|data|sealed|inline|\
                                enum|annotation|actual|expect|companion|value|final|operator|infix|tailrec|external";

/// Line pattern for a file extension. `None` = no outline for this type.
fn pattern(ext: &str) -> Option<String> {
    let p = match ext {
        "md" | "markdown" => r"^#{1,6}\s+\S".to_string(),
        "rs" => r#"^\s*(pub(\([^)]*\))?\s+)?((async|const|unsafe|default|extern(\s+"[^"]*")?)\s+)*(fn|struct|enum|trait|impl|mod|type|union)\b|^\s*macro_rules!"#
            .to_string(),
        "py" => r"^\s*(async\s+)?(def|class)\s".to_string(),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => {
            r"^\s*(export\s+)?(default\s+)?(declare\s+)?(abstract\s+)?(async\s+)?(function\b|class\s|interface\s|type\s+\w+.*=|enum\s)|^(export\s+)?(const|let|var)\s+\w+[^=]*=\s*(async\s*)?(function\b|\([^)]*\)\s*(:[^=]+)?=>|\w+\s*=>)"
                .to_string()
        }
        "kt" | "kts" => format!(r"^\s*(({KOTLIN_MODIFIERS})\s+)*(fun|class|interface|object|typealias)\b"),
        "java" => r"^\s*((public|private|protected|static|final|abstract|sealed)\s+)*(class|interface|enum|record)\s|^\s*(public|private|protected)\s[^=;]*\([^;]*$"
            .to_string(),
        "go" => r"^(func|type)\s".to_string(),
        "sh" | "bash" => r"^\s*(function\s+[\w:-]+|[\w:-]+\s*\(\)\s*\{?\s*$)".to_string(),
        "toml" | "ini" => r"^\s*\[".to_string(),
        _ => return None,
    };
    Some(p)
}

/// `[{line, text}]` (1-based lines) plus whether the list was cut at the cap.
/// Empty for unknown file types and for files without a match.
pub fn outline(path: &Path, content: &str) -> (Vec<Value>, bool) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let Some(re) = pattern(&ext).and_then(|p| Regex::new(&p).ok()) else {
        return (Vec::new(), false);
    };
    let markdown = matches!(ext.as_str(), "md" | "markdown");
    let mut entries = Vec::new();
    let mut in_fence = false;
    for (i, line) in content.lines().enumerate() {
        if markdown {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
        }
        if !re.is_match(line) {
            continue;
        }
        if entries.len() == MAX_ENTRIES {
            return (entries, true);
        }
        let text: String = line.trim().chars().take(MAX_ENTRY_CHARS).collect();
        entries.push(json!({"line": i + 1, "text": text}));
    }
    (entries, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(name: &str, content: &str) -> Vec<String> {
        let (entries, _) = outline(Path::new(name), content);
        entries
            .iter()
            .map(|e| format!("{}:{}", e["line"], e["text"].as_str().unwrap()))
            .collect()
    }

    #[test]
    fn markdown_headings_outside_fences() {
        let md = "# Title\ntext\n```sh\n# a comment\n```\n## Part\n#hashtag\n";
        assert_eq!(texts("n.md", md), ["1:# Title", "6:## Part"]);
    }

    #[test]
    fn code_definitions() {
        let rs = "use x;\npub async fn run() {\n    let f = 1;\n}\nimpl Foo for Bar {\npub(crate) struct S;\n";
        assert_eq!(
            texts("a.rs", rs),
            [
                "2:pub async fn run() {",
                "5:impl Foo for Bar {",
                "6:pub(crate) struct S;"
            ]
        );
        let py = "import os\nclass A:\n    async def go(self):\n        pass\ndefault = 1\n";
        assert_eq!(texts("a.py", py), ["2:class A:", "3:async def go(self):"]);
        let ts = "import x from 'y'\nexport const add = (a: number) => a\nconst n = 1\nexport default class K {\nfunction f() {}\n";
        assert_eq!(
            texts("a.ts", ts),
            [
                "2:export const add = (a: number) => a",
                "4:export default class K {",
                "5:function f() {}"
            ]
        );
        let kt = "package p\nprivate suspend fun load() {\nval x = 1\ndata class P(val a: Int)\n";
        assert_eq!(
            texts("a.kt", kt),
            [
                "2:private suspend fun load() {",
                "4:data class P(val a: Int)"
            ]
        );
        let go = "package main\nfunc main() {\n\tfunc() {}()\n}\ntype T struct{}\n";
        assert_eq!(texts("a.go", go), ["2:func main() {", "5:type T struct{}"]);
        let sh = "#!/bin/sh\ndie() {\n  exit 1\n}\nfunction run {\n";
        assert_eq!(texts("a.sh", sh), ["2:die() {", "5:function run {"]);
        let java =
            "package p;\npublic class A {\n  private int n = f();\n  public int get(int a) {\n";
        assert_eq!(
            texts("A.java", java),
            ["2:public class A {", "4:public int get(int a) {"]
        );
    }

    #[test]
    fn unknown_type_and_cap() {
        assert!(texts("a.txt", "# not a heading here\n").is_empty());
        let many = "# h\n".repeat(MAX_ENTRIES + 5);
        let (entries, cut) = outline(Path::new("big.md"), &many);
        assert_eq!((entries.len(), cut), (MAX_ENTRIES, true));
    }
}
