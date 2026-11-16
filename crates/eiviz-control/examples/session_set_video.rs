//! Builds a session whose first scene shows a video file, for decode benchmarks:
//!   session_set_video <in.eivz> <out.eivz> <video-path>
use eiviz_control::session::file::{read_document, write_document};
use eiviz_control::session::{InputKind, SceneLayer};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: session_set_video <in.eivz> <out.eivz> <video>");
        std::process::exit(2);
    }
    let mut document = read_document(&args[1]).expect("read session");
    let video = document
        .inputs
        .iter_mut()
        .find(|input| input.kind == InputKind::Video)
        .expect("session has a video input");
    video.path_or_address = Some(args[3].clone());
    video.video_loop = true;
    let video_id = video.id;
    let scene_id = document.scenes[0].id;
    let layer: SceneLayer = serde_json::from_str(&format!(
        r#"{{"inputId": {video_id}, "width": 1.0, "height": 1.0}}"#
    ))
    .expect("layer");
    document.scenes[0].layers = vec![layer];
    for unit in &mut document.units {
        unit.program_scene_id = scene_id;
        unit.preview_scene_id = scene_id;
    }
    write_document(&args[2], &document).expect("write session");
    println!("wrote {} (video id {video_id}, scene {scene_id})", args[2]);
}
