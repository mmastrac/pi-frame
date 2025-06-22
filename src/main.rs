use std::path::Path;

use gstreamer::{Bin, GhostPad, prelude::*};
use gstreamer_video::VideoInfo;
use serde::Deserialize;

fn stream_rtsp(
    url: &str,
    width: usize,
    height: usize,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::new();
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    rtspsrc location={url} 
        ! queue ! rtph264depay ! h264parse ! queue ! v4l2h264dec 
        ! queue 
        ! videoscale 
        ! videoconvert 
        ! video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1
        ! queue name=sink
    "#
    ))?;
    bin.add(&pipeline)?;

    let sink = pipeline.downcast::<gstreamer::Bin>().expect("not a bin");
    let sink = sink.by_name("sink").expect("no sink");
    let sink_pad = sink.static_pad("src").expect("static pad");

    let ghost_pad = GhostPad::with_target(&sink_pad)?;
    ghost_pad.set_active(true)?;
    bin.add_pad(&ghost_pad)?;
    Ok(bin.upcast())
}

fn stream_image(
    image: &str,
    width: usize,
    height: usize,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::new();
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    filesrc location={image} 
        ! decodebin
        ! imagefreeze name="image"
        ! videobox name="padding" autocrop=true
        ! videoscale
        ! videoconvert 
        ! video/x-raw,width={width},height={height}
        ! queue name=sink
    "#
    ))?;
    bin.add(&pipeline)?;

    let sink = pipeline.downcast::<gstreamer::Bin>().expect("not a bin");
    let sink = sink.by_name("sink").expect("no sink");
    let sink_pad = sink.static_pad("src").expect("static pad");

    let image = bin.by_name("image").expect("no image");
    let image_pad = image.static_pad("src").expect("no src");
    image_pad.add_probe(gstreamer::PadProbeType::BUFFER, move |pad, _buffer| {
        if let Some(caps) = pad.current_caps() {
            if let Ok(vinfo) = VideoInfo::from_caps(&caps) {
                println!(
                    "Image bounds: {}x{}, format: {}",
                    vinfo.width(),
                    vinfo.height(),
                    vinfo.format()
                );
            }
        }
        gstreamer::PadProbeReturn::Remove.into()
    });

    let ghost_pad = GhostPad::with_target(&sink_pad)?;
    ghost_pad.set_active(true)?;
    bin.add_pad(&ghost_pad)?;
    Ok(bin.upcast())
}

#[derive(Debug)]
struct CompositorPad {
    pad: gstreamer::Pad,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn make_compositor(
    width: usize,
    height: usize,
) -> Result<(gstreamer::Element, Vec<CompositorPad>), Box<dyn std::error::Error>> {
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    compositor name="mixer" background="transparent" 
        ! queue ! videoconvert ! queue ! fbdevsink sync=false
    "#
    ))?;
    let pipeline = pipeline.downcast::<gstreamer::Bin>().expect("not a bin");
    let compositor = pipeline.by_name("mixer").expect("no mixer");
    let mut pads = vec![];

    for n in 0..4 {
        let pad = compositor
            .request_pad_simple(&format!("sink_{n}"))
            .expect("no pad");
        let ghost = GhostPad::with_target(&pad)?;
        ghost.set_active(true)?;
        pipeline.add_pad(&ghost)?;
        pads.push(CompositorPad {
            pad,
            x: ((n % 2) * width / 2) as _,
            y: ((n / 2) * height / 2) as _,
            width: (width / 2) as _,
            height: (height / 2) as _,
        });
    }

    Ok((pipeline.upcast(), pads))
}
#[derive(Debug, Deserialize)]
struct Source {
    description: String,
    #[serde(flatten)]
    source: SourceType,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SourceType {
    Rtsp { rtsp: String },
    Videotestsrc { videotestsrc: String },
    Image { image: String },
}

#[derive(Debug, Deserialize)]
struct Config {
    display: Display,
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
struct Display {
    framebuffer: String,
    layout: Layout,
}

#[derive(Debug, Deserialize)]
struct Layout {
    horizontal: usize,
    vertical: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_file = std::env::args()
        .nth(1)
        .expect("Config file must be the first argument");
    let config_file = Path::new(&config_file).canonicalize()?;
    let config_dir = config_file.parent().unwrap().to_owned();
    let config = toml::from_str::<Config>(std::fs::read_to_string(config_file)?.as_str())?;

    let mut framebuffer = framebuffer::Framebuffer::new(&config.display.framebuffer)?;
    let (width, height) = (
        framebuffer.var_screen_info.xres,
        framebuffer.var_screen_info.yres,
    );
    eprintln!("Framebuffer size: {width}x{height}");

    let frame = framebuffer.read_frame();
    let zeros = vec![0; frame.len()];
    framebuffer.write_frame(&zeros);

    eprintln!("Config:");
    eprintln!("{config:?}");

    // Set up main loop
    let main_loop = glib::MainLoop::new(None, false);

    // Initialize GStreamer
    gstreamer::init()?;

    let (compositor, pads) = make_compositor(1280, 800)?;

    let pipeline = gstreamer::Pipeline::with_name("pi-frame");
    pipeline.add(&compositor)?;

    for (index, source) in config.sources.iter().enumerate() {
        let element = match &source.source {
            SourceType::Rtsp { rtsp } => {
                eprintln!("Configuring RTSP source: {rtsp}");
                let stream = stream_rtsp(&rtsp, 640, 400)?;
                stream
            }
            SourceType::Videotestsrc { videotestsrc } => {
                eprintln!("Configuring videotestsrc source: {videotestsrc}");
                let stream = gstreamer::ElementFactory::make("videotestsrc")
                    .property_from_str("pattern", &videotestsrc)
                    .build()?;
                stream
            }
            SourceType::Image { image } => {
                let image = config_dir.join(image).canonicalize()?;
                eprintln!("Configuring image source: {image:?}");
                let stream = stream_image(image.to_str().unwrap(), 640, 400)?;
                stream
            }
        };

        let text_overlay = gstreamer::ElementFactory::make("textoverlay").build()?;
        text_overlay.set_property("text", &source.description);
        text_overlay.set_property("font-desc", "Arial 20");
        pipeline.add(&text_overlay)?;
        pipeline.add(&element)?;

        element.link(&text_overlay)?;

        let pad = compositor
            .static_pad(&format!("sink_{index}"))
            .expect("no pad sink_{index}");
        text_overlay.static_pad("src").expect("no src").link(&pad)?;
    }

    for pad in pads {
        eprintln!("{pad:?}");
        pad.pad.set_property("xpos", pad.x);
        pad.pad.set_property("ypos", pad.y);
        pad.pad.set_property("width", pad.width);
        pad.pad.set_property("height", pad.height);
    }

    pipeline.set_state(gstreamer::State::Playing)?;

    main_loop.run();

    Ok(())
}
