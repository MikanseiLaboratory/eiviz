//! Prints the audio state of a session file, for E2E verification:
//!   cargo run -p eiviz-control --example session_dump -- /path/session.eivz
use eiviz_control::session::file::read_document;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: session_dump <session.eivz>");
        std::process::exit(2);
    };
    let document = match read_document(&path) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("read {path}: {error}");
            std::process::exit(1);
        }
    };
    for input in &document.inputs {
        println!(
            "input\t{}\t{}\tgain={:.4}\tmute={}\tunits={:?}",
            input.id, input.name, input.gain, input.mute, input.audio_units
        );
    }
    for unit in &document.units {
        println!(
            "unit\t{}\t{}\tgain={:.4}\tmute={}\tkind={:?}\tdevice={}",
            unit.id,
            unit.name,
            unit.audio.gain,
            unit.audio.mute,
            unit.audio.device_kind,
            unit.audio.device_id
        );
    }
    for output in &document.outputs {
        println!(
            "output\t{}\t{}\ttransport={:?}\tenabled={}",
            output.id, output.name, output.transport, output.enabled
        );
    }
    for input in &document.inputs {
        println!("order\t{}\t{}", input.id, input.name);
    }
}
