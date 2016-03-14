use std::cell::RefCell;
use std::cmp::max;
use std::f32::consts::PI;
use std::f32::INFINITY;
use std::fs::File;
use std::path::{ Path, PathBuf };
use std::rc::Rc;

use array::*;
use camera_controllers::{ CameraPerspective, FirstPerson, FirstPersonSettings };
use docopt;
use flate2::read::GzDecoder;
use fps_counter::{ FPSCounter };
use gfx::handle::Buffer;
use gfx::traits::Device;
use gfx;
use gfx_device_gl;
use piston::event_loop::{ Events, EventLoop };
use piston::input::Event;
use piston::window::{ Size, Window, AdvancedWindow, OpenGLWindow, WindowSettings };
use sdl2_window::Sdl2Window;
use time;
use vecmath::*;

use minecraft;
use minecraft::assets::Assets;
use minecraft::biome::Biomes;
use minecraft::block_state::BlockStates;
use minecraft::nbt::Nbt;
use minecraft::region::Region;

use chunk::{ BiomeId, Chunk, ChunkManager };
use player::Player;
use shader::{ Renderer, Vertex };

pub static USAGE: &'static str = "
hematite, Minecraft made in Rust!

Usage:
    hematite [options] <world>

Options:
    -p, --path               Fully qualified path for world folder.
    --mcversion=<version>    Minecraft version [default: 1.8.8].
";

#[derive(RustcDecodable, Clone, Debug)]
pub struct Args {
    arg_world: String,
    flag_path: bool,
    flag_mcversion: String,
}

pub struct App<'a, R: gfx::Resources, F: gfx::Factory<R>, D: gfx::Device> where R: 'a {
    pub args: Args,
    pub assets: Assets<R>,
    pub camera: RefCell<FirstPerson>,
    pub capture_cursor: RefCell<bool>,
    pub chunk_manager: RefCell<ChunkManager<R>>,
    pub device: RefCell<D>,
    pub fps_counter: RefCell<FPSCounter>,
    pub pending_chunks: RefCell<Vec<(Vector3<i32>, &'a RefCell<Option<Buffer<R, Vertex>>>, 
        [[[&'a Chunk; 3]; 3]; 3], Matrix3<Option<&'a [[BiomeId; 16]; 16]>>)>>,
    pub player: RefCell<Player>,
    pub region: RefCell<Region>,
    pub region_path: RefCell<PathBuf>,
    pub renderer: RefCell<Renderer<R, F>>,
    pub staging_buffer: RefCell<Vec<Vertex>>,
    pub window: RefCell<Sdl2Window>,
    pub world: Nbt,
    pub world_path: PathBuf,
}

impl<'a> App<'a, gfx_device_gl::Resources, gfx_device_gl::Factory, gfx_device_gl::Device> {

    pub fn from_args(args: Args) -> Self {
        // Automagically pull MC assets
        minecraft::fetch_assets(&args.flag_mcversion);

        // Automagically expand path if world is located at
        // $MINECRAFT_ROOT/saves/<world_name>
        let world = if args.flag_path {
            PathBuf::from(&args.arg_world)
        } else {
            let mut mc_path = minecraft::vanilla_root_path();
            mc_path.push("saves");
            mc_path.push(args.arg_world.clone());
            mc_path
        };

        let file_name = PathBuf::from(world.join("level.dat"));
        let level_reader = GzDecoder::new(File::open(file_name).unwrap()).unwrap();
        let level = minecraft::nbt::Nbt::from_reader(level_reader).unwrap();
        println!("{:?}", level);

        let player = Player::from_nbt(&level);

        let player_chunk = [player.pos.x(), player.pos.z()]
            .map(|x| (x / 16.0).floor() as i32);

        let regions = player_chunk.map(|x| x >> 5);
        let region_file = world.join(
                format!("region/r.{}.{}.mca", regions[0], regions[1])
            );
        let region = minecraft::region::Region::open(&region_file).unwrap();

        let loading_title = format!(
                "Hematite loading... - {}",
                world.file_name().unwrap().to_str().unwrap()
            );

        let mut window: Sdl2Window = WindowSettings::new(
                loading_title,
                Size { width: 854, height: 480 })
                .fullscreen(false)
                .exit_on_esc(true)
                .samples(0)
                .vsync(false)
                .build()
                .unwrap();

        let (mut device, mut factory) = gfx_device_gl::create(|s|
            window.get_proc_address(s) as *const _
        );

        let Size { width: w, height: h } = window.size();

        let (target_view, depth_view) = gfx_device_gl::create_main_targets(
            (w as u16, h as u16, 1, (0 as gfx::tex::NumSamples).into()));

        let assets = Path::new("./assets");

        // Load biomes.
        let biomes = Biomes::load(&assets);

        // Load block state definitions and models.
        let block_states = BlockStates::load(&assets, &mut factory);

        let assets = Assets {
            biomes: biomes,
            block_states: block_states,
        };

        let mut renderer = Renderer::new(factory, target_view, depth_view, 
            assets.block_states.texture.surface.clone());

        let projection_mat = CameraPerspective {
            fov: 70.0,
            near_clip: 0.1,
            far_clip: 1000.0,
            aspect_ratio: {
                let Size { width: w, height: h } = window.size();
                (w as f32) / (h as f32)
            }
        }.projection();
        renderer.set_projection(projection_mat);

        let mut first_person_settings = FirstPersonSettings::keyboard_wasd();
        first_person_settings.speed_horizontal = 8.0;
        first_person_settings.speed_vertical = 4.0;
        let mut first_person = FirstPerson::new(
            player.pos,
            first_person_settings
        );
        first_person.yaw = PI - player.yaw / 180.0 * PI;
        first_person.pitch = player.pitch / 180.0 * PI;

        App {
            args: args,
            assets: assets,
            camera: RefCell::new(first_person),
            capture_cursor: RefCell::new(false),
            chunk_manager: RefCell::new(ChunkManager::new()),
            device: RefCell::new(device),
            fps_counter: RefCell::new(FPSCounter::new()),
            pending_chunks: RefCell::new(vec![]),
            player: RefCell::new(player),
            region: RefCell::new(region),
            region_path: RefCell::new(region_file),
            renderer: RefCell::new(renderer),
            staging_buffer: RefCell::new(vec![]),
            window: RefCell::new(window),
            world: level,
            world_path: world,
        }
    }

    pub fn handle_event(&self, event: Event) {
        use piston::input::Button::Keyboard;
        use piston::input::Input::{ Move, Press };
        use piston::input::keyboard::Key;
        use piston::input::Motion::MouseRelative;
        use piston::input::Event;

        match event {
            Event::Render(_) => {
                self.render();
            }
            Event::AfterRender(_) => {
                self.device.borrow_mut().cleanup();
            }
            Event::Update(_) => {
                use std::i32;
                // HACK(eddyb) find the closest chunk to the player.
                // The pending vector should be sorted instead.
                let pp = self.camera.borrow().position.map(|x| (x / 16.0).floor() as i32);
                let closest = self.pending_chunks.borrow().iter().enumerate().fold(
                    (None, i32::max_value()),
                    |(best_i, best_dist), (i, &(cc, _, _, _))| {
                        let xyz = [cc[0] - pp[0], cc[1] - pp[1], cc[2] - pp[2]]
                            .map(|x| x * x);
                        let dist = xyz[0] + xyz[1] + xyz[2];
                        if dist < best_dist {
                            (Some(i), dist)
                        } else {
                            (best_i, best_dist)
                        }
                    }
                ).0;
                let pending = closest.and_then(|i| {
                    // Vec swap_remove doesn't return Option anymore
                    match self.pending_chunks.borrow().len() {
                        0 => None,
                        _ => Some(self.pending_chunks.borrow_mut().swap_remove(i))
                    }
                });
                match pending {
                    Some((coords, buffer, chunks, column_biomes)) => {
                        minecraft::block_state::fill_buffer(
                            &self.assets, &mut *self.staging_buffer.borrow_mut(),
                            coords, chunks, column_biomes
                        );
                        *buffer.borrow_mut() = Some(
                            self.renderer.borrow_mut().create_buffer(&self.staging_buffer.borrow_mut()[..])
                        );
                        self.staging_buffer.borrow_mut().clear();

                        if self.pending_chunks.borrow().is_empty() {
                            println!("Finished filling chunk vertex buffers.");
                        }
                    }
                    None => {}
                }
            }
            Event::Input(Press(Keyboard(Key::C))) => {
                let mut capture_cursor = self.capture_cursor.borrow_mut();
                println!("Turned cursor capture {}",
                    if *capture_cursor { "off" } else { "on" });
                *capture_cursor = !*capture_cursor;

                self.window.borrow_mut().set_capture_cursor(*capture_cursor);
            }
            Event::Input(Move(MouseRelative(_, _))) => {
                if !*self.capture_cursor.borrow() {
                    // Don't send the mouse event to the FPS controller.
                    return;
                }
            }
            _ => {}
        }

        self.camera.borrow_mut().event(&event);
    }

    pub fn render(&self) {
        // Apply the same y/z camera offset vanilla minecraft has.
        let mut camera = self.camera.borrow().camera(0.0);
        camera.position[1] += 1.62;
        let mut xz_forward = camera.forward;
        xz_forward[1] = 0.0;
        xz_forward = vec3_normalized(xz_forward);
        camera.position = vec3_add(
            camera.position,
            vec3_scale(xz_forward, 0.1)
        );

        let view_mat = camera.orthogonal();
        self.renderer.borrow_mut().set_view(view_mat);
        self.renderer.borrow_mut().clear();
        let mut num_chunks: usize = 0;
        let mut num_sorted_chunks: usize = 0;
        let mut num_total_chunks: usize = 0;
        let start_time = time::precise_time_ns();
        self.chunk_manager.borrow_mut().each_chunk(|cx, cy, cz, _, buffer| {
            if let Some(buffer) = buffer.borrow_mut().as_mut() {
                num_total_chunks += 1;

                let inf = INFINITY;
                let mut bb_min = [inf, inf, inf];
                let mut bb_max = [-inf, -inf, -inf];
                let xyz = [cx, cy, cz].map(|x| x as f32 * 16.0);
                for &dx in [0.0, 16.0].iter() {
                    for &dy in [0.0, 16.0].iter() {
                        for &dz in [0.0, 16.0].iter() {
                            use vecmath::col_mat4_transform;

                            let v = vec3_add(xyz, [dx, dy, dz]);
                            let xyzw = col_mat4_transform(view_mat, [v[0], v[1], v[2], 1.0]);
                            let v = col_mat4_transform(self.renderer.borrow().get_projection(), xyzw);
                            let xyz = vec3_scale([v[0], v[1], v[2]], 1.0 / v[3]);
                            bb_min = Array::from_fn(|i| bb_min[i].min(xyz[i]));
                            bb_max = Array::from_fn(|i| bb_max[i].max(xyz[i]));
                        }
                    }
                }

                let cull_bits: [bool; 3] = Array::from_fn(|i| {
                    let (min, max) = (bb_min[i], bb_max[i]);
                    min.signum() == max.signum()
                        && min.abs().min(max.abs()) >= 1.0
                });

                if !cull_bits.iter().any(|&cull| cull) {
                    self.renderer.borrow_mut().render(buffer);
                    num_chunks += 1;

                    if bb_min[0] < 0.0 && bb_max[0] > 0.0
                    || bb_min[1] < 0.0 && bb_max[1] > 0.0 {
                        num_sorted_chunks += 1;
                    }
                }
            }
        });
        let end_time = time::precise_time_ns();
        self.renderer.borrow_mut().flush(&mut *self.device.borrow_mut());
        let frame_end_time = time::precise_time_ns();

        let fps = self.fps_counter.borrow_mut().tick();
        let title = format!(
                "Hematite sort={} render={} total={} in {:.2}ms+{:.2}ms @ {}FPS - {}",
                num_sorted_chunks,
                num_chunks,
                num_total_chunks,
                (end_time - start_time) as f64 / 1e6,
                (frame_end_time - end_time) as f64 / 1e6,
                fps, self.world_path.file_name().unwrap().to_str().unwrap()
            );
        self.window.borrow_mut().set_title(title);
    }
}

impl<'a, R: gfx::Resources, F: gfx::Factory<R>, D: gfx::Device> App<'a, R, F, D> {

    pub fn load_chunks(&'a self) {
        let player = self.player.borrow();
        let player_chunk = [player.pos.x(), player.pos.z()]
            .map(|x| (x / 16.0).floor() as i32);

        let regions = player_chunk.map(|x| x >> 5);
        let c_bases = player_chunk.map(|x| max(0, (x & 0x1f) - 8) as u8);


        self.chunk_manager.borrow().each_chunk_and_neighbors(
            |coords, buffer, chunks, column_biomes| {
                self.pending_chunks.borrow_mut().push((coords, buffer, chunks, column_biomes).clone());
            }
        );

        for cz in c_bases[1]..c_bases[1] + 16 {
            for cx in c_bases[0]..c_bases[0] + 16 {
                match self.region.borrow().get_chunk_column(cx, cz) {
                    Some(column) => {
                        let (cx, cz) = (
                            cx as i32 + regions[0] * 32,
                            cz as i32 + regions[1] * 32
                        );
                        self.chunk_manager.borrow_mut().add_chunk_column(cx, cz, column)
                    }
                    None => {}
                }
            }
        }
    }
}