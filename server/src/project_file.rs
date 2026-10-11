//! Project memory: `<project dir>/.taskbot.md` (docs/protocol.md "Project
//! memory"). The file is the store, read fresh on every use. The server
//! changes bullet lines under `## Pinned` and `## Tasks` only; every other
//! byte stays as the user or the agent left it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const FILE_NAME: &str = ".taskbot.md";
/// Bigger files are not read and not written.
pub const MAX_BYTES: usize = 64 * 1024;
pub const PINNED: &str = "## Pinned";
pub const TASKS: &str = "## Tasks";
/// `## Tasks` lines kept.
pub const TASKS_KEEP: usize = 20;
/// Put on top of a `delegate` prompt when the project has the file.
pub const NOTES_LINE: &str = "Project notes kept by taskbot: .taskbot.md. Read it first.";

/// One read-change-write at a time per file.
static LOCKS: Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// The file. `Ok(None)` = no file. `Err` = there, but not usable: not a plain
/// file, not UTF-8, over [`MAX_BYTES`].
pub fn read(dir: &Path) -> Result<Option<String>, String> {
    let path = path(dir);
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let too_big = || format!("{} is over 64 KiB", path.display());
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err(too_big());
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.len() > MAX_BYTES {
        return Err(too_big());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{} is not UTF-8", path.display()))
}

/// [`read`] for the prompt side: a file that cannot be used counts as no file.
pub fn load(dir: &Path) -> Option<String> {
    read(dir).unwrap_or_else(|e| {
        tracing::warn!("project memory skipped: {e}");
        None
    })
}

/// Epoch ms of the last write to the file, 0 = unknown.
pub fn modified_ms(dir: &Path) -> i64 {
    std::fs::metadata(path(dir))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// The first `limit` chars of `text`, cut back to a line end.
pub fn cut(text: &str, limit: usize) -> &str {
    let Some((end, _)) = text.char_indices().nth(limit) else {
        return text;
    };
    let head = &text[..end];
    match head.rfind('\n') {
        Some(nl) => &head[..=nl],
        None => head,
    }
}

/// Where a section sits, as line numbers.
struct Section {
    heading: usize,
    bullets: Vec<usize>,
}

/// Section `name` of the file: from its heading to the next `#` or `##`
/// heading. Lines inside a code fence are neither headings nor bullets.
fn section(lines: &[&str], name: &str) -> Option<Section> {
    let mut fenced = false;
    let mut found: Option<Section> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        match &mut found {
            None if line.trim_end() == name => {
                found = Some(Section {
                    heading: i,
                    bullets: Vec::new(),
                })
            }
            None => {}
            Some(_) if line.starts_with("# ") || line.starts_with("## ") => break,
            Some(s) if line.starts_with("- ") => s.bullets.push(i),
            Some(_) => {}
        }
    }
    found
}

/// Bullet texts of section `name`, top to bottom. No section: none.
pub fn bullets(text: &str, name: &str) -> Vec<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    section(&lines, name)
        .map(|s| s.bullets)
        .unwrap_or_default()
        .into_iter()
        .map(|i| lines[i][2..].trim().to_string())
        .collect()
}

/// One change to the bullets of a section. Numbers count its bullets from 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edit<'a> {
    /// New bullet below the others.
    Add(&'a str),
    /// New bullet above the others.
    AddOnTop(&'a str),
    Swap(usize, &'a str),
    Remove(usize),
}

fn end_line(line: &mut String) {
    if !line.ends_with('\n') {
        line.push('\n');
    }
}

/// `text` with one bullet of section `name` added, swapped or removed. A
/// missing section is added at the end of the file. A bullet number past the
/// end changes nothing.
pub fn edit(text: &str, name: &str, edit: Edit) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let found = section(&lines, name);
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match (edit, found) {
        (Edit::Add(bullet) | Edit::AddOnTop(bullet), None) => {
            if let Some(last) = out.last_mut() {
                end_line(last);
                if !last.trim().is_empty() {
                    out.push("\n".into());
                }
            }
            out.push(format!("{name}\n"));
            out.push(format!("- {bullet}\n"));
        }
        (Edit::Add(bullet) | Edit::AddOnTop(bullet), Some(s)) => {
            let at = match edit {
                Edit::Add(_) => s.bullets.last().map_or(s.heading + 1, |i| i + 1),
                _ => s.bullets.first().copied().unwrap_or(s.heading + 1),
            };
            // Only the last line of a file can lack its line end.
            end_line(&mut out[at - 1]);
            out.insert(at, format!("- {bullet}\n"));
        }
        (Edit::Swap(n, bullet), Some(s)) => {
            if let Some(&i) = s.bullets.get(n) {
                let ending = &lines[i][lines[i].trim_end_matches(['\n', '\r']).len()..];
                out[i] = format!("- {bullet}{ending}");
            }
        }
        (Edit::Remove(n), Some(s)) => {
            if let Some(&i) = s.bullets.get(n) {
                out.remove(i);
            }
        }
        (Edit::Swap(..) | Edit::Remove(_), None) => {}
    }
    out.concat()
}

fn one_line(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .filter(|c| c.is_whitespace() || !c.is_control())
        .collect();
    let line = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    line.chars().take(max).collect::<String>().trim_end().into()
}

/// A `## Tasks` bullet: `<date> <prompt, 80 chars> -> done: <answer, 120 chars>`
/// or `-> failed: <error, 120 chars>`. `detail` = the answer or the error.
pub fn task_line(date: &str, prompt: &str, done: bool, detail: Option<&str>) -> String {
    let outcome = if done { "done" } else { "failed" };
    let detail = one_line(detail.unwrap_or(""), 120);
    let mut line = format!("{date} {} -> {outcome}", one_line(prompt, 80));
    if !detail.is_empty() {
        line.push_str(": ");
        line.push_str(&detail);
    }
    line
}

/// `text` with a task line on top of `## Tasks`, the oldest lines past
/// [`TASKS_KEEP`] dropped.
pub fn with_task(text: &str, line: &str) -> String {
    let mut text = edit(text, TASKS, Edit::AddOnTop(line));
    loop {
        let n = bullets(&text, TASKS).len();
        if n <= TASKS_KEEP {
            return text;
        }
        text = edit(&text, TASKS, Edit::Remove(n - 1));
    }
}

fn lock(file: &Path) -> Arc<Mutex<()>> {
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(file.to_path_buf()).or_default().clone()
}

/// Temp file in the same folder, then rename: a reader never sees half a file.
fn write(dir: &Path, text: &str) -> Result<(), String> {
    let path = path(dir);
    let tmp = dir.join(format!(
        "{FILE_NAME}.{}.tmp",
        hex::encode(crate::random_bytes::<4>())
    ));
    let written = std::fs::write(&tmp, text)
        .and_then(|()| match std::fs::metadata(&path) {
            // The file keeps the mode its owner gave it.
            Ok(old) => std::fs::set_permissions(&tmp, old.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| std::fs::rename(&tmp, &path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })
}

/// Read, change, write under the lock of the project. `change` gets the file
/// as it is right now (no file: the `# Taskbot: <project>` top line it would
/// start with) and returns the new text, or `None` to leave it alone: then no
/// file is made either. `Ok` = the file afterwards, `None` = still no file.
pub fn update(
    dir: &Path,
    project: &str,
    change: impl FnOnce(&str) -> Option<String>,
) -> Result<Option<String>, String> {
    let lock = lock(&path(dir));
    let _one_at_a_time = lock.lock().unwrap_or_else(|e| e.into_inner());
    let old = read(dir)?;
    let new = match &old {
        Some(text) => change(text),
        None => change(&format!("# Taskbot: {project}\n")),
    };
    let Some(new) = new else {
        return Ok(old);
    };
    if new.len() > MAX_BYTES {
        return Err(format!("{} would pass 64 KiB", path(dir).display()));
    }
    write(dir, &new)?;
    Ok(Some(new))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# Taskbot: garden

Free text by the user.

## Pinned
- notes go under beds/
- budget max 200 EUR

## Layout
- not ours
```
## Pinned
- inside a fence
```

## Tasks
- 2026-10-12 split plan.md -> done: 4 files

### sub heading stays in the section
- 2026-10-11 price list -> failed: gone
";

    #[test]
    fn bullets_of_a_section_only() {
        assert_eq!(
            bullets(FILE, PINNED),
            ["notes go under beds/", "budget max 200 EUR"]
        );
        assert_eq!(
            bullets(FILE, TASKS),
            [
                "2026-10-12 split plan.md -> done: 4 files",
                "2026-10-11 price list -> failed: gone"
            ]
        );
        assert!(bullets("# x\n- loose\n", PINNED).is_empty());
        assert!(bullets("", TASKS).is_empty());
    }

    /// Lines `a` has and `b` lacks, and the other way round.
    fn line_diff(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
        let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
        let only = |x: &[&str], y: &[&str]| {
            x.iter()
                .filter(|l| !y.contains(l))
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        };
        (only(&la, &lb), only(&lb, &la))
    }

    #[test]
    fn each_edit_touches_one_line() {
        let added = edit(FILE, PINNED, Edit::Add("water at dawn"));
        assert_eq!(
            line_diff(FILE, &added),
            (vec![], vec!["- water at dawn".to_string()])
        );
        assert_eq!(
            bullets(&added, PINNED),
            [
                "notes go under beds/",
                "budget max 200 EUR",
                "water at dawn"
            ],
            "below the others"
        );
        assert!(added.contains("- budget max 200 EUR\n- water at dawn\n\n## Layout\n"));

        let swapped = edit(FILE, PINNED, Edit::Swap(1, "budget max 300 EUR"));
        assert_eq!(
            line_diff(FILE, &swapped),
            (
                vec!["- budget max 200 EUR".to_string()],
                vec!["- budget max 300 EUR".to_string()]
            )
        );
        assert_eq!(swapped.len(), FILE.len(), "in place");

        let removed = edit(FILE, PINNED, Edit::Remove(0));
        assert_eq!(
            line_diff(FILE, &removed),
            (vec!["- notes go under beds/".to_string()], vec![])
        );
        // Everything outside the section is byte-equal.
        for changed in [&added, &swapped, &removed] {
            let (head, tail) = (
                "# Taskbot: garden\n\nFree text by the user.\n\n## Pinned\n",
                &FILE[FILE.find("\n## Layout").unwrap()..],
            );
            assert!(changed.starts_with(head) && changed.ends_with(tail));
        }
        // A number past the end changes nothing.
        assert_eq!(edit(FILE, PINNED, Edit::Remove(2)), FILE);
        assert_eq!(edit(FILE, PINNED, Edit::Swap(9, "x")), FILE);
        assert_eq!(edit("# x\n", PINNED, Edit::Remove(0)), "# x\n");

        let top = edit(FILE, TASKS, Edit::AddOnTop("2026-10-13 new -> done"));
        assert_eq!(
            bullets(&top, TASKS)[0],
            "2026-10-13 new -> done",
            "newest first"
        );
        assert!(top.contains("## Tasks\n- 2026-10-13 new -> done\n- 2026-10-12 split"));
    }

    #[test]
    fn missing_section_goes_to_the_end() {
        assert_eq!(
            edit("# Taskbot: p\n", PINNED, Edit::Add("a")),
            "# Taskbot: p\n\n## Pinned\n- a\n"
        );
        // No line end on the last line, CRLF kept on a swap, empty section.
        assert_eq!(
            edit("# p\nfree", TASKS, Edit::AddOnTop("t")),
            "# p\nfree\n\n## Tasks\n- t\n"
        );
        assert_eq!(
            edit("## Pinned\r\n- a\r\n", PINNED, Edit::Swap(0, "b")),
            "## Pinned\r\n- b\r\n"
        );
        assert_eq!(
            edit("## Pinned", PINNED, Edit::Add("a")),
            "## Pinned\n- a\n"
        );
        assert_eq!(
            edit("## Pinned\n\n## Tasks\n- t\n", PINNED, Edit::Add("a")),
            "## Pinned\n- a\n\n## Tasks\n- t\n"
        );
        assert_eq!(edit("", PINNED, Edit::Add("a")), "## Pinned\n- a\n");
    }

    #[test]
    fn task_lines_roll_at_twenty() {
        assert_eq!(
            task_line(
                "2026-10-12",
                "split\n plan.md\tper bed",
                true,
                Some("4 files")
            ),
            "2026-10-12 split plan.md per bed -> done: 4 files"
        );
        assert_eq!(
            task_line("2026-10-11", "p", false, Some("agent session is gone")),
            "2026-10-11 p -> failed: agent session is gone"
        );
        assert_eq!(
            task_line("2026-10-11", "p", true, None),
            "2026-10-11 p -> done"
        );
        let long = task_line("d", &"p".repeat(200), true, Some(&"é".repeat(500)));
        assert_eq!(long.chars().count(), 2 + 80 + " -> done: ".len() + 120);

        let mut text = "# p\n\n## Pinned\n- keep\n".to_string();
        for i in 1..=21 {
            text = with_task(&text, &format!("task {i}"));
        }
        let tasks = bullets(&text, TASKS);
        assert_eq!(tasks.len(), TASKS_KEEP);
        assert_eq!(
            (tasks[0].as_str(), tasks[19].as_str()),
            ("task 21", "task 2")
        );
        assert_eq!(bullets(&text, PINNED), ["keep"]);
    }

    #[test]
    fn cut_ends_on_a_line() {
        assert_eq!(cut("ab\ncd\n", 100), "ab\ncd\n");
        assert_eq!(cut("ab\ncd\nef", 5), "ab\n");
        assert_eq!(cut("ab\ncd\nef", 6), "ab\ncd\n");
        assert_eq!(cut("éééééé", 3), "ééé", "one huge line: plain cut");
    }

    #[test]
    fn update_makes_the_file_on_the_first_write_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), Ok(None));
        // Leaving it alone makes no file.
        assert_eq!(update(dir.path(), "p", |_| None), Ok(None));
        assert!(!path(dir.path()).exists());

        let made = update(dir.path(), "p", |old| {
            Some(edit(old, PINNED, Edit::Add("a")))
        });
        assert_eq!(made.unwrap().unwrap(), "# Taskbot: p\n\n## Pinned\n- a\n");
        assert_eq!(
            read(dir.path()).unwrap().unwrap(),
            "# Taskbot: p\n\n## Pinned\n- a\n"
        );
        assert!(modified_ms(dir.path()) > 1_600_000_000_000);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|f| f.unwrap().file_name())
            .collect();
        assert_eq!(names, [FILE_NAME], "no temp file left");

        // The mode the owner gave the file survives a write.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path(dir.path()), std::fs::Permissions::from_mode(0o600)).unwrap();
        update(dir.path(), "p", |old| Some(format!("{old}more\n"))).unwrap();
        let mode = std::fs::metadata(path(dir.path())).unwrap().permissions();
        assert_eq!(mode.mode() & 0o777, 0o600);
    }

    #[test]
    fn unusable_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path(dir.path()), [0xff, 0xfe, b'a']).unwrap();
        assert!(read(dir.path()).unwrap_err().contains("not UTF-8"));
        assert_eq!(load(dir.path()), None);
        let e = update(dir.path(), "p", |_| Some("x".into())).unwrap_err();
        assert!(e.contains("not UTF-8"), "{e}");
        assert_eq!(
            std::fs::read(path(dir.path())).unwrap(),
            [0xff, 0xfe, b'a'],
            "left alone"
        );

        std::fs::write(path(dir.path()), "x".repeat(MAX_BYTES + 1)).unwrap();
        assert!(read(dir.path()).unwrap_err().contains("64 KiB"));
        std::fs::write(path(dir.path()), "x".repeat(MAX_BYTES)).unwrap();
        assert!(read(dir.path()).unwrap().is_some());
        let e = update(dir.path(), "p", |old| Some(format!("{old}y"))).unwrap_err();
        assert!(e.contains("64 KiB"), "{e}");
    }
}
