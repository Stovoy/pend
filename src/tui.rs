//! Super-minimal interactive view of all jobs in the directory.
//!
//! *Non-blocking nice-to-have* – provides a quick overview similar to `top`.

use std::io::{self, Write};

use crate::paths::jobs_root;

use crossterm::{cursor, event, execute, style, terminal, ExecutableCommand};

/// Entry point called from `main.rs` when the `tui` subcommand is used.
pub(crate) fn run_tui() -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let res = (|| -> io::Result<()> {
        loop {
            // Handle input – exit on 'q' or Ctrl-C.
            while event::poll(std::time::Duration::from_millis(100))? {
                if let event::Event::Key(key) = event::read()? {
                    if key.code == event::KeyCode::Char('q') || key.code == event::KeyCode::Esc {
                        return Ok(());
                    }
                }
            }

            // Gather job info.
            let root = jobs_root()?;
            let mut jobs: Vec<(String, String)> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if let Some((job, ext)) = name.rsplit_once('.') {
                            if matches!(ext, "out" | "err" | "log" | "exit" | "json" | "signal" | "lock") {
                                jobs.push((job.to_string(), ext.to_string()));
                            }
                        }
                    }
                }
            }
            jobs.sort_by(|a, b| a.0.cmp(&b.0));
            // Deduplicate by job name.
            let mut unique: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for (job, _) in jobs {
                unique.insert(job);
            }

            // Render
            let mut y = 0;
            stdout.execute(cursor::MoveTo(0, 0))?;
            stdout.execute(terminal::Clear(terminal::ClearType::All))?;
            writeln!(stdout, "press 'q' to quit\n")?;
            y += 2;

            for job in unique {
                let exit_path = root.join(format!("{job}.exit"));
                let status = if exit_path.exists() {
                    let code = std::fs::read_to_string(exit_path)?.trim().to_string();
                    format!("exit {code}")
                } else {
                    "running".into()
                };
                stdout.execute(cursor::MoveTo(0, y))?;
                stdout.execute(style::Print(format!("{:<20} {}", job, status)))?;
                y += 1;
            }
            stdout.flush()?;
        }
    })();

    // Restore terminal state.
    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    res
}
