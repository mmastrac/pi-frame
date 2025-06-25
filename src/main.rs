use std::{collections::HashMap, path::Path};

use gstreamer::{Bin, GhostPad, MessageView, prelude::*};
use gstreamer_video::VideoInfo;
use serde::Deserialize;

const RTSP_PREFIX: &str = "rtsp_";

fn stream_rtsp(
    url: &str,
    id: &str,
    width: usize,
    height: usize,
    scale: RtspScale,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::with_name(id);
    let (scale, scale_opts) = match scale {
        RtspScale::Fit => (String::new(), ""),
        RtspScale::Crop => (
            format!("! aspectratiocrop aspect-ratio={width}/{height}"),
            "",
        ),
        RtspScale::Scale => (format!(""), "add-borders=false"),
    };

    // Buffer up to 2 seconds of video with a target latency of 200ms
    let id = format!("{RTSP_PREFIX}{id}");
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    rtspsrc location={url:?} name={id:?} latency=2000 drop-on-latency=true protocols=udp
        ! queue leaky=downstream
        ! rtph264depay ! h264parse 
        ! queue leaky=downstream
        ! v4l2h264dec name="decoder"
        {scale} 
        ! queue leaky=downstream max-size-time=2000000000
        ! videoconvertscale  {scale_opts}
        ! video/x-raw,width={width},height={height},pixel-aspect-ratio=1/1
        ! queue name=sink
    "#
    ))?;
    bin.add(&pipeline)?;

    let decoder = bin.by_name("decoder").expect("no decoder");
    probe_image_format("decoder", &decoder.static_pad("src").expect("no src"));

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
    scale: Option<(usize, usize)>,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::new();
    let scale = if let Some((width, height)) = scale {
        format!("! videoscale ! video/x-raw,width={width},height={height}")
    } else {
        String::new()
    };
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    filesrc location={image} 
        ! decodebin
        {scale}
        ! imagefreeze name="image"
        ! videorate ! video/x-raw,framerate=1/1
        ! videobox name="padding" autocrop=true
        ! videoscale
        ! videoconvert 
        ! video/x-raw,width={width},height={height}
        ! queue max-size-buffers=1 leaky=downstream name=sink
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

fn stream_videotestsrc(
    pattern: &str,
    width: usize,
    height: usize,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let bin = Bin::new();
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    videotestsrc pattern={pattern}
        ! queue
        ! videoscale
        ! videoconvert
        ! video/x-raw,width={width},height={height}
        ! queue max-size-buffers=1 leaky=downstream name=sink
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
    layout: Layout,
    time: Option<String>,
) -> Result<(gstreamer::Element, Vec<CompositorPad>), Box<dyn std::error::Error>> {
    let time = time
        .map(|time| {
            format!(
                r#"! clockoverlay halignment=right valignment=bottom
                    time-format={time:?} font-desc="Arial 8"
                    halignment=absolute valignment=absolute
                    x-absolute=1 y-absolute=1
                    "#,
            )
        })
        .unwrap_or_default();
    let pipeline = gstreamer::parse::launch(&format!(
        r#"
    compositor name="mixer"
        ! videorate drop-only=true
        ! videoconvert
        ! video/x-raw,framerate=10/1,width={width},height={height},pixel-aspect-ratio=1/1
        {time}
        ! fbdevsink sync=false
    "#
    ))?;
    let pipeline = pipeline.downcast::<gstreamer::Bin>().expect("not a bin");
    let compositor = pipeline.by_name("mixer").expect("no mixer");
    let mut pads = vec![];

    let compositor_pad = compositor.static_pad("src").expect("no src");
    probe_image_format("compositor out", &compositor_pad);

    for n in 0..layout.horizontal * layout.vertical {
        let pad = compositor
            .request_pad_simple(&format!("sink_{n}"))
            .expect("no pad");

        probe_image_format("compositor", &pad);

        let ghost = GhostPad::with_target(&pad)?;
        ghost.set_active(true)?;
        pipeline.add_pad(&ghost)?;
        pads.push(CompositorPad {
            pad,
            x: ((n % layout.horizontal) * width / layout.horizontal) as _,
            y: ((n / layout.horizontal) * height / layout.vertical) as _,
            width: (width / layout.horizontal) as _,
            height: (height / layout.vertical) as _,
        });
    }

    Ok((pipeline.upcast(), pads))
}

#[derive(Debug, Deserialize, Clone)]
struct Source {
    description: String,
    #[serde(flatten)]
    source: SourceType,
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RtspScale {
    /// Show bars on the sides of the video
    Fit,
    /// Crop the video to the aspect ratio of the container
    Crop,
    /// Scale the video to the aspect ratio of the container
    Scale,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
enum SourceType {
    Rtsp {
        rtsp: String,
        scale: RtspScale,
    },
    Videotestsrc {
        videotestsrc: String,
    },
    Image {
        image: String,
        width: Option<usize>,
        height: Option<usize>,
    },
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    display: Display,
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize, Clone)]
struct Display {
    framebuffer: String,
    layout: Layout,
    time: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
struct Layout {
    horizontal: usize,
    vertical: usize,
}

/// A source that has been instantiated and added to the pipeline
struct InstantiatedSource {
    source: Source,
    name: String,
    index: usize,
    width: usize,
    height: usize,
}

fn restart_source(
    pipeline: &gstreamer::Pipeline,
    source: &InstantiatedSource,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Restarting source: {}", source.name);

    let bin = pipeline
        .by_name(&source.name)
        .expect("no bin")
        .downcast::<gstreamer::Bin>()
        .expect("not a bin");

    // Get the bin's output pad so we can figure out what it was linked to
    let pad = bin.static_pad("src").expect("no src");
    let peer = pad.peer().expect("no peer");
    pad.unlink(&peer)?;
    let peer_parent = peer
        .parent()
        .expect("no parent")
        .downcast::<gstreamer::Element>()
        .expect("not an element");
    peer_parent.release_request_pad(&peer);
    pipeline.remove(&bin)?;

    // "Can't set the state of the src to NULL from its streaming thread"
    // https://github.com/GStreamer/gst-python/blob/master/examples/dynamic_src.py
    glib::idle_add(move || {
        match bin.set_state(gstreamer::State::Null) {
            Ok(_) => eprintln!("Set bin to null"),
            Err(e) => eprintln!("Error setting bin to null: {e:?}"),
        }
        glib::ControlFlow::Break
    });

    let element = create_source(source)?;
    pipeline.add(&element)?;

    // Link the new element to the peer pad
    element.link(&peer_parent)?;
    element.sync_state_with_parent()?;

    println!("Restarted source: {}", source.name);
    Ok(())
}

fn create_source(
    source: &InstantiatedSource,
) -> Result<gstreamer::Element, Box<dyn std::error::Error>> {
    let stream = match &source.source.source {
        SourceType::Rtsp { rtsp, scale } => {
            eprintln!("Configuring RTSP source: {rtsp}");
            let stream = stream_rtsp(&rtsp, &source.name, source.width, source.height, *scale)?;
            stream
        }
        SourceType::Videotestsrc { videotestsrc } => {
            eprintln!("Configuring videotestsrc source: {videotestsrc}");
            let stream = stream_videotestsrc(&videotestsrc, source.width, source.height)?;
            stream
        }
        SourceType::Image {
            image,
            width: scale_width,
            height: scale_height,
        } => {
            eprintln!("Configuring image source: {image:?}");
            let scale = match (*scale_width, *scale_height) {
                (Some(width), Some(height)) => Some((width, height)),
                (None, None) => None,
                _ => panic!("width and height must be provided"),
            };
            let stream = stream_image(&image, source.width, source.height, scale)?;
            stream
        }
    };
    Ok(stream)
}

fn probe_image_format(name: &str, pad: &gstreamer::Pad) {
    let name = name.to_string();
    pad.add_probe(gstreamer::PadProbeType::BUFFER, move |pad, _buffer| {
        if let Some(caps) = pad.current_caps() {
            if let Ok(vinfo) = VideoInfo::from_caps(&caps) {
                println!(
                    "Image bounds for {}: {}x{}, format: {}",
                    name,
                    vinfo.width(),
                    vinfo.height(),
                    vinfo.format()
                );
            }
        }
        gstreamer::PadProbeReturn::Remove.into()
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_file = std::env::args()
        .nth(1)
        .expect("Config file must be the first argument");
    let config_file = Path::new(&config_file).canonicalize()?;
    let config_dir = config_file.parent().unwrap().to_owned();
    let mut config = toml::from_str::<Config>(std::fs::read_to_string(config_file)?.as_str())?;

    // Resolve image paths
    for source in &mut config.sources {
        match &mut source.source {
            SourceType::Image { image, .. } => {
                *image = config_dir
                    .join(&image)
                    .canonicalize()?
                    .to_str()
                    .expect("image path is not valid")
                    .to_string();
            }
            _ => {}
        }
    }

    let mut framebuffer = framebuffer::Framebuffer::new(&config.display.framebuffer)?;
    let (width, height) = (
        framebuffer.var_screen_info.xres,
        framebuffer.var_screen_info.yres,
    );
    eprintln!("Framebuffer size: {width}x{height}");

    // Clear the framebuffer in debug mode
    if std::env::var("CLEAR_FRAMEBUFFER").is_ok() {
        let frame = framebuffer.read_frame();
        let zeros = vec![0; frame.len()];
        framebuffer.write_frame(&zeros);
    }

    eprintln!("Config:");
    eprintln!("{config:?}");

    // Set up main loop
    let main_loop = glib::MainLoop::new(None, false);

    // Initialize GStreamer
    gstreamer::init()?;

    let (compositor, pads) = make_compositor(
        width as _,
        height as _,
        config.display.layout,
        config.display.time,
    )?;

    let pipeline = gstreamer::Pipeline::with_name("pi-frame");
    pipeline.add(&compositor)?;

    let mut sources = HashMap::new();

    for (index, source) in config.sources.into_iter().enumerate() {
        let name = format!("src_{}", index);
        let instantiated_source = InstantiatedSource {
            source: source.clone(),
            name: name.clone(),
            index,
            width: (width as usize / config.display.layout.horizontal),
            height: (height as usize / config.display.layout.vertical),
        };

        let element = create_source(&instantiated_source)?;
        sources.insert(name, instantiated_source);

        pipeline.add(&element)?;

        let text_overlay = gstreamer::parse::launch(&format!(
            r#"textoverlay text={:?} font-desc="Arial 20" scale-mode="none""#,
            source.description
        ))?;
        pipeline.add(&text_overlay)?;
        element.link(&text_overlay)?;

        let pad = compositor
            .static_pad(&format!("sink_{index}"))
            .expect("no pad sink_{index}");
        text_overlay.static_pad("src").expect("no src").link(&pad)?;
    }

    for pad in pads {
        pad.pad.set_property("xpos", pad.x);
        pad.pad.set_property("ypos", pad.y);
        pad.pad.set_property("width", pad.width);
        pad.pad.set_property("height", pad.height);
    }

    let pipeline_clone = pipeline.clone();
    let _guard = pipeline.bus().unwrap().add_watch(move |_, msg| {
        match msg.view() {
            MessageView::Error(err) => {
                println!("Error: {}: {err:?}", err.error());

                if let Some(structure) = err.structure() {
                    if structure.name() == "GstMessageError" {
                        if let Some(source) = err.src() {
                            let source_name = source.name().to_string();
                            println!("Error from source: {source_name}");
                            if source_name.starts_with(RTSP_PREFIX) {
                                let name = source_name.strip_prefix(RTSP_PREFIX).unwrap();
                                let source = sources.get(name).unwrap();
                                if let Err(e) = restart_source(&pipeline_clone, source) {
                                    eprintln!("*** Failed to restart source {source_name}: {e:?}");
                                }
                            }
                        }
                    }
                }
            }
            MessageView::StateChanged(state) => {
                // Check for interesting state changes: rtspsrc*, pi-frame
                if let Some(src) = state.src() {
                    let name = src.name();
                    if name.starts_with(RTSP_PREFIX) || name == "pi-frame" {
                        if state.old() != gstreamer::State::Null {
                            println!(
                                "State changed [{name:?}]: {:?} -> {:?}",
                                state.old(),
                                state.current()
                            );
                        }
                    }
                }
            }
            MessageView::Element(element) => {
                if let Some(structure) = element.structure() {
                    if structure.name() == "GstRTSPSrcTimeout" {
                        if let Some(src) = element.src() {
                            let name = src.name().to_string();
                            println!("RTSP timeout on source: {name}");
                            let name = name.strip_prefix(RTSP_PREFIX).unwrap();
                            let source = sources.get(name).unwrap();
                            if let Err(e) = restart_source(&pipeline_clone, source) {
                                eprintln!("*** Failed to restart source {name}: {e:?}");
                            }
                        }
                    } else if structure.name().contains("Timeout") {
                        println!("Timeout on element: {:?}", element);
                    }
                }
            }
            MessageView::StreamStatus(status) => {
                if let Some(_src) = status.src() {
                    if let Some(structure) = status.structure() {
                        if let Ok(status_type) = structure.value("type") {
                            // Coercse status_type to String
                            let status_type_string = format!("{:?}", status_type);
                            if status_type_string.contains("GST_STREAM_STATUS_TYPE_CREATE")
                                || status_type_string.contains("GST_STREAM_STATUS_TYPE_ENTER")
                                || status_type_string.contains("GST_STREAM_STATUS_TYPE_LEAVE")
                            {
                                // ignore
                            } else {
                                println!("Stream status: {:?}", structure);
                            }
                        }
                    }
                }
            }
            MessageView::Eos(element) => {
                println!("EOS on element: {:?}", element);
            }
            _ => {
                // println!("Message: {:?}", msg.view());
            }
        }
        glib::ControlFlow::Continue
    })?;
    pipeline.set_state(gstreamer::State::Playing)?;

    main_loop.run();

    Ok(())
}
