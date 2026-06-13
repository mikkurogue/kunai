pub mod dashboard;
pub mod list;
pub mod wizard;

use std::io::Write;

pub fn clear_inline(count: u16) {
    for _ in 1..count {
        print!("\x1b[2K\x1b[1A");
    }
    print!("\x1b[2K\r");
    std::io::stdout().flush().ok();
}
