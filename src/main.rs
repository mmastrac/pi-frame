use gstreamer::{Bin, GhostPad, prelude::*};

fn stream_rtsp(
    url: &str,
    width: usize,
    height: usize,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::new();
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    rtspsrc location={url} ! queue ! rtph264depay ! queue 
    ! h264parse ! queue ! v4l2h264dec ! queue ! videoscale ! queue 
    ! videoconvert ! video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1
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
    compositor name="mixer" ! queue ! videoconvert ! queue ! fbdevsink sync=false
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up main loop
    let main_loop = glib::MainLoop::new(None, false);

    // Initialize GStreamer
    gstreamer::init()?;

    let (compositor, pads) = make_compositor(1280, 800)?;

    let pipeline = gstreamer::Pipeline::with_name("pi-frame");

    let videotestsrc = gstreamer::ElementFactory::make("videotestsrc").build()?;
    let stream_rtsp1 = stream_rtsp("rtsp://192.168.1.1:7447/lU7RqYAF3W9ZborZ", 640, 400)?;
    let stream_rtsp2 = stream_rtsp("rtsp://192.168.1.1:7447/IjIY1LIA3w5k2XW3", 640, 400)?;
    let stream_rtsp3 = stream_rtsp("rtsp://192.168.1.1:7447/XzM3XEhXCY2jNB8G", 640, 400)?;

    pipeline.add_many([
        &videotestsrc,
        &stream_rtsp1,
        &stream_rtsp2,
        &stream_rtsp3,
        &compositor,
    ])?;
    let pad = compositor.static_pad("sink_0").expect("no pad sink_0");
    stream_rtsp1.static_pad("src").expect("no src").link(&pad)?;
    let pad = compositor.static_pad("sink_1").expect("no pad sink_1");
    stream_rtsp2.static_pad("src").expect("no src").link(&pad)?;
    let pad = compositor.static_pad("sink_2").expect("no pad sink_2");
    stream_rtsp3.static_pad("src").expect("no src").link(&pad)?;
    let pad = compositor.static_pad("sink_3").expect("no pad sink_3");
    videotestsrc.static_pad("src").expect("no src").link(&pad)?;

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
