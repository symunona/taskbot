//! Read-only access to the vault. Every path is canonicalised (symlinks
//! resolved) and must stay below the canonical vault root.

use regex::RegexBuilder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 50;
const MAX_MATCH_CHARS: usize = 300;
const MAX_LIST_ITEMS: usize = 200;
/// One `read` page. A file within both limits is returned whole.
const PAGE_LINES: usize = 300;
const PAGE_CHARS: usize = 20_000;

#[derive(Clone)]
pub struct Vault {
    root: PathBuf,
}

fn hidden(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| n.starts_with('.'))
}

impl Vault {
    /// `root` must exist; it is canonicalised once here.
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("vault_path {}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("vault_path {} is not a directory", root.display()));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Map a vault-relative path to a real path inside the vault.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        if rel.contains('\0') {
            return Err("invalid path".into());
        }
        if Path::new(rel).is_absolute() {
            return Err("path must be relative to the vault root, without a leading '/'".into());
        }
        // canonicalize() resolves `..` and every symlink, so the prefix check
        // below is against where the path really points.
        let real = self
            .root
            .join(rel)
            .canonicalize()
            .map_err(|_| format!("not found: {rel}"))?;
        if !real.starts_with(&self.root) {
            return Err(format!("path is outside the vault: {rel}"));
        }
        Ok(real)
    }

    pub fn read_file(&self, rel: &str) -> Result<String, String> {
        let path = self.resolve(rel)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("{rel}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("not a file: {rel}"));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("file too large ({} bytes): {rel}", meta.len()));
        }
        std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))
    }

    /// `read_file` tool. No range: whole file when small, else its outline, else the
    /// first page. With a range: those lines, cut at the page cap.
    pub fn read(&self, rel: &str, start: Option<i64>, end: Option<i64>) -> Result<Value, String> {
        let content = self.read_file(rel)?;
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if start.is_none() && end.is_none() {
            if total <= PAGE_LINES && content.chars().count() <= PAGE_CHARS {
                return Ok(json!({ "content": content }));
            }
            let (entries, cut) = crate::outline::outline(Path::new(rel), &content);
            if !entries.is_empty() {
                let mut out = json!({"outline": entries, "lines": total});
                if cut {
                    out["outline_truncated"] = json!(true);
                }
                return Ok(out);
            }
        }
        let first = start.unwrap_or(1);
        if first < 1 || first as usize > total.max(1) {
            return Err(format!(
                "start_line {first} is outside the file (1..{total})"
            ));
        }
        let first = first as usize;
        let last = match end {
            Some(e) if e < first as i64 => {
                return Err(format!("end_line {e} is before start_line {first}"))
            }
            Some(e) => (e as usize).min(total),
            None => total,
        };
        let mut page = String::new();
        let mut chars = 0;
        let mut taken = 0;
        for line in lines.iter().take(last).skip(first - 1) {
            let n = line.chars().count() + 1;
            // Always take one line, so a single over-long line cannot stall paging.
            if taken == PAGE_LINES || (taken > 0 && chars + n > PAGE_CHARS) {
                break;
            }
            page.push_str(line);
            page.push('\n');
            chars += n;
            taken += 1;
        }
        let end_line = first + taken.max(1) - 1;
        let mut out =
            json!({"content": page, "start_line": first, "end_line": end_line, "lines": total});
        if end_line < last {
            out["next_line"] = json!(end_line + 1);
        }
        Ok(out)
    }

    /// `{items}`: entry names, sorted; directories carry a trailing `/`. Dot-entries are
    /// skipped. Over the cap: first `MAX_LIST_ITEMS` plus `truncated` and `total`.
    pub fn list_directory(&self, rel: &str) -> Result<Value, String> {
        let path = self.resolve(rel)?;
        if !path.is_dir() {
            return Err(format!("not a directory: {rel}"));
        }
        let mut items = Vec::new();
        for entry in std::fs::read_dir(&path)
            .map_err(|e| format!("{rel}: {e}"))?
            .flatten()
        {
            let name = entry.file_name();
            if hidden(&name) {
                continue;
            }
            let mut name = name.to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                name.push('/');
            }
            items.push(name);
        }
        items.sort();
        let total = items.len();
        if total <= MAX_LIST_ITEMS {
            return Ok(json!({ "items": items }));
        }
        items.truncate(MAX_LIST_ITEMS);
        Ok(json!({"items": items, "truncated": true, "total": total}))
    }

    /// One result per matching file: `{path, match}` with the first matching line.
    /// Symlinks are not followed, so the walk cannot leave the vault.
    pub fn search_regexp(&self, pattern: &str) -> Result<Value, String> {
        let re = RegexBuilder::new(pattern)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| format!("bad pattern: {e}"))?;
        let mut results = Vec::new();
        let mut truncated = false;
        let walk = WalkDir::new(&self.root).sort_by_file_name().into_iter();
        for entry in walk
            .filter_entry(|e| e.depth() == 0 || !hidden(e.file_name()))
            .flatten()
        {
            if !entry.file_type().is_file()
                || entry.metadata().map_or(true, |m| m.len() > MAX_FILE_BYTES)
            {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Some(m) = re.find(&content) else { continue };
            if results.len() == MAX_SEARCH_RESULTS {
                truncated = true;
                break;
            }
            let start = content[..m.start()].rfind('\n').map_or(0, |i| i + 1);
            let line = content[start..].lines().next().unwrap_or("");
            let line: String = line.chars().take(MAX_MATCH_CHARS).collect();
            let rel = entry
                .path()
                .strip_prefix(&self.root)
                .unwrap_or(entry.path());
            results.push(json!({"path": rel.to_string_lossy(), "match": line}));
        }
        let mut out = json!({ "results": results });
        if truncated {
            out["truncated"] = json!(true);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// tmp/outside/secret.txt, tmp/vault/{a.md, sub/b.md, .hidden/c.md, link -> ../outside, leak.md -> ../outside/secret.txt, inner -> sub}
    fn fixture() -> (tempfile::TempDir, Vault) {
        let tmp = tempfile::tempdir().unwrap();
        let v = tmp.path().join("vault");
        fs::create_dir_all(v.join("sub")).unwrap();
        fs::create_dir_all(v.join(".hidden")).unwrap();
        fs::create_dir_all(tmp.path().join("outside")).unwrap();
        fs::write(tmp.path().join("outside/secret.txt"), "TOPSECRET needle").unwrap();
        fs::write(v.join("a.md"), "alpha\nthe needle is here\nomega").unwrap();
        fs::write(v.join("sub/b.md"), "beta").unwrap();
        fs::write(v.join(".hidden/c.md"), "needle hidden").unwrap();
        std::os::unix::fs::symlink("../outside", v.join("link")).unwrap();
        std::os::unix::fs::symlink("../outside/secret.txt", v.join("leak.md")).unwrap();
        std::os::unix::fs::symlink("sub", v.join("inner")).unwrap();
        let vault = Vault::new(&v).unwrap();
        (tmp, vault)
    }

    #[test]
    fn reads_inside() {
        let (_t, v) = fixture();
        assert_eq!(v.read_file("sub/b.md").unwrap(), "beta");
        assert_eq!(v.read_file("./sub/../sub/b.md").unwrap(), "beta");
        assert_eq!(
            v.read_file("inner/b.md").unwrap(),
            "beta",
            "symlink staying inside is fine"
        );
        assert_eq!(
            v.list_directory("").unwrap(),
            json!({"items": ["a.md", "inner", "leak.md", "link", "sub/"]})
        );
        assert_eq!(v.list_directory("sub").unwrap(), json!({"items": ["b.md"]}));
    }

    #[test]
    fn rejects_dotdot() {
        let (_t, v) = fixture();
        for p in [
            "../outside/secret.txt",
            "../../x",
            "sub/../../outside/secret.txt",
            "..",
            "sub/../..",
        ] {
            assert!(v.read_file(p).is_err(), "{p}");
            assert!(v.list_directory(p).is_err(), "{p}");
        }
    }

    #[test]
    fn rejects_absolute() {
        let (t, v) = fixture();
        let outside = t.path().join("outside/secret.txt");
        assert!(v.read_file(outside.to_str().unwrap()).is_err());
        assert!(v.read_file("/etc/passwd").is_err());
        assert!(v.list_directory("/").is_err());
        // Even an absolute path that points into the vault: paths are vault-relative.
        assert!(v
            .read_file(v.root().join("a.md").to_str().unwrap())
            .is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let (_t, v) = fixture();
        assert!(v.read_file("leak.md").is_err());
        assert!(v.read_file("link/secret.txt").is_err());
        assert!(v.list_directory("link").is_err());
        assert!(v.read("leak.md", None, None).is_err());
        assert!(v.read("../outside/secret.txt", Some(1), Some(2)).is_err());
    }

    #[test]
    fn search_stays_inside() {
        let (_t, v) = fixture();
        let out = v.search_regexp("needle").unwrap();
        assert_eq!(
            out["results"],
            json!([{"path": "a.md", "match": "the needle is here"}])
        );
        assert!(v.search_regexp("TOPSECRET").unwrap()["results"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(v.search_regexp("(").is_err());
    }

    #[test]
    fn small_file_comes_whole() {
        let (_t, v) = fixture();
        assert_eq!(
            v.read("a.md", None, None).unwrap(),
            json!({"content": "alpha\nthe needle is here\nomega"})
        );
        assert_eq!(
            v.read("a.md", Some(2), Some(2)).unwrap(),
            json!({"content": "the needle is here\n", "start_line": 2, "end_line": 2, "lines": 3})
        );
        assert_eq!(v.read("a.md", Some(3), None).unwrap()["content"], "omega\n");
        assert_eq!(v.read("a.md", None, Some(99)).unwrap()["end_line"], 3);
        for (s, e) in [
            (Some(0), None),
            (Some(4), None),
            (Some(3), Some(2)),
            (Some(-1), Some(2)),
        ] {
            assert!(v.read("a.md", s, e).is_err(), "{s:?} {e:?}");
        }
    }

    #[test]
    fn big_file_gives_outline_then_pages() {
        let (_t, v) = fixture();
        let mut md = String::from("# Top\n");
        for i in 0..400 {
            md.push_str(&format!("line {i}\n"));
        }
        md.push_str("## Tail\nend\n");
        fs::write(v.root().join("big.md"), &md).unwrap();
        assert_eq!(
            v.read("big.md", None, None).unwrap(),
            json!({"outline": [{"line": 1, "text": "# Top"}, {"line": 402, "text": "## Tail"}], "lines": 403})
        );
        let page = v.read("big.md", Some(1), None).unwrap();
        assert_eq!(
            (&page["end_line"], &page["next_line"]),
            (&json!(300), &json!(301))
        );
        assert_eq!(page["content"].as_str().unwrap().lines().count(), 300);
        let rest = v.read("big.md", Some(301), None).unwrap();
        assert_eq!(rest["end_line"], 403);
        assert!(rest.get("next_line").is_none());
        assert_eq!(
            v.read("big.md", Some(402), Some(403)).unwrap()["content"],
            "## Tail\nend\n"
        );

        // No outline for this type: first page instead.
        fs::write(v.root().join("big.txt"), &md).unwrap();
        let page = v.read("big.txt", None, None).unwrap();
        assert_eq!(
            (&page["start_line"], &page["next_line"]),
            (&json!(1), &json!(301))
        );
    }

    #[test]
    fn page_is_cut_by_chars_and_long_lines_still_advance() {
        let (_t, v) = fixture();
        let line = "x".repeat(PAGE_CHARS + 10);
        fs::write(v.root().join("wide.txt"), format!("{line}\nshort\n")).unwrap();
        let page = v.read("wide.txt", None, None).unwrap();
        assert_eq!(
            (&page["end_line"], &page["next_line"]),
            (&json!(1), &json!(2))
        );
        assert_eq!(
            v.read("wide.txt", Some(2), None).unwrap()["content"],
            "short\n"
        );
    }

    #[test]
    fn listing_is_capped() {
        let (_t, v) = fixture();
        fs::create_dir(v.root().join("many")).unwrap();
        for i in 0..MAX_LIST_ITEMS + 3 {
            fs::write(v.root().join(format!("many/{i:04}.md")), "").unwrap();
        }
        let out = v.list_directory("many").unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), MAX_LIST_ITEMS);
        assert_eq!(
            (&out["truncated"], &out["total"]),
            (&json!(true), &json!(MAX_LIST_ITEMS + 3))
        );
    }
}
