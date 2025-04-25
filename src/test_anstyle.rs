#[allow(dead_code)]
fn _t() {
    use anstyle::{AnsiColor, Style};
    let style = Style::new().fg_color(Some(AnsiColor::Red.into()));
    let text = style.render_owned("hello");
    let _ = text;
}
