extern crate byteorder;
extern crate camera_controllers;
extern crate docopt;
extern crate flate2;
extern crate fps_counter;
#[macro_use] extern crate gfx;
extern crate gfx_device_gl;
extern crate gfx_voxel;
extern crate sdl2_window;
extern crate image;
extern crate libc;
extern crate memmap;
extern crate piston;
extern crate rustc_serialize;
extern crate shader_version;
extern crate time;
extern crate vecmath;
extern crate zip;

// Reexport modules from gfx_voxel while stuff is moving
// from Hematite to the library.
pub use gfx_voxel::{ array, cube };

use std::cell::RefCell;
use std::cmp::max;
use std::fs::File;
use std::path::{ Path, PathBuf };
use std::rc::Rc;

use array::*;
use docopt::Docopt;
use flate2::read::GzDecoder;
use gfx::traits::Device;
use piston::event_loop::{ Events, EventLoop };
use piston::window::{ Size, Window, AdvancedWindow, OpenGLWindow, WindowSettings };
use sdl2_window::Sdl2Window;
use shader::Renderer;
use vecmath::*;

pub mod app;
pub mod chunk;
pub mod minecraft;
pub mod player;
pub mod shader;

use minecraft::biome::Biomes;
use minecraft::block_state::BlockStates;

fn main() {
    let args: app::Args = Docopt::new(app::USAGE)
                                 .and_then(|dopt| dopt.decode())
                                 .unwrap_or_else(|e| e.exit());

    let app = app::App::from_args(args);

    println!("Started loading chunks...");
    app.load_chunks();
    println!("Finished loading chunks.");

    println!("Press C to capture mouse");

    let mut events = app.window.events().ups(120).max_fps(10_000);
    while let Some(e) = events.next(&mut app.window) {
        app.handle_event(e);
    }
}
