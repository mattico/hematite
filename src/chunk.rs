use std::cell::RefCell;
use std::cmp::max;
use std::collections::HashMap;
use std::path::{ Path, PathBuf };

use array::*;
use shader::Vertex;
use gfx;
use vecmath::*;

use minecraft;
use minecraft::assets::Assets;
use minecraft::block_state::BlockStates;
use minecraft::region::Region;
use player::Player;

#[derive(Copy, Clone)]
pub struct BlockState {
    pub value: u16
}

pub const EMPTY_BLOCK: BlockState = BlockState { value: 0 };

#[derive(Copy, Clone)]
pub struct BiomeId {
    pub value: u8
}

#[derive(Copy, Clone)]
pub struct LightLevel {
    pub value: u8
}

impl LightLevel {
    pub fn block_light(self) -> u8 {
        self.value & 0xf
    }
    pub fn sky_light(self) -> u8 {
        self.value >> 4
    }
}

pub const SIZE: usize = 16;

/// A chunk of SIZE x SIZE x SIZE blocks, in YZX order.
#[derive(Copy, Clone)]
pub struct Chunk {
    pub blocks: [[[BlockState; SIZE]; SIZE]; SIZE],
    pub light_levels: [[[LightLevel; SIZE]; SIZE]; SIZE]
}

// TODO: Change to const pointer.
pub const EMPTY_CHUNK: &'static Chunk = &Chunk {
    blocks: [[[EMPTY_BLOCK; SIZE]; SIZE]; SIZE],
    light_levels: [[[LightLevel {value: 0xf0}; SIZE]; SIZE]; SIZE]
};

pub struct ChunkColumn<R: gfx::Resources> {
    pub chunks: Vec<Chunk>,
    pub buffers: [RefCell<Option<gfx::handle::Buffer<R, Vertex>>>; SIZE],
    pub biomes: [[BiomeId; SIZE]; SIZE]
}

pub struct ChunkBuffer<'a, R: gfx::Resources> where R: 'a {
    pub coords: Vector3<i32>,
    pub buffer: &'a Option<gfx::handle::Buffer<R, Vertex>>,
    pub chunks: [[[&'a Chunk; 3]; 3]; 3],
    pub biomes: Matrix3<Option<&'a [[BiomeId; 16]; 16]>>,
}

pub struct ChunkManager<'a, R: gfx::Resources> where R: 'a {
    chunk_columns: HashMap<(i32, i32), ChunkColumn<R>>,
    pending_chunks: Vec<ChunkBuffer<'a, R>>,
    region: Region,
    region_path: PathBuf,
}

impl<'a, R: gfx::Resources> ChunkManager<'a, R> {
    pub fn open(path: &Path) -> ChunkManager<'a, R> {
        ChunkManager {
            chunk_columns: HashMap::new(),
            pending_chunks: Vec::new(),
            region: Region::open(path).unwrap(),
            region_path: path.to_path_buf(),
        }
    }

    pub fn add_chunk_column(&mut self, x: i32, z: i32, c: ChunkColumn<R>) {
        self.chunk_columns.insert((x, z), c);
    }
    
    pub fn load_chunks(&'a mut self, player: &Player) {
        let player_chunk = [player.pos.x(), player.pos.z()]
            .map(|x| (x / 16.0).floor() as i32);

        let regions = player_chunk.map(|x| x >> 5);
        let c_bases = player_chunk.map(|x| max(0, (x & 0x1f) - 8) as u8);


        self.each_chunk_and_neighbors(
            |coords, buffer, chunks, column_biomes| {
                self.pending_chunks.push(ChunkBuffer {
                    coords: coords,
                    buffer: buffer,
                    chunks: chunks,
                    biomes: column_biomes,
                });
            }
        );

        for cz in c_bases[1]..c_bases[1] + 16 {
            for cx in c_bases[0]..c_bases[0] + 16 {
                match self.region.get_chunk_column(cx, cz) {
                    Some(column) => {
                        let (cx, cz) = (
                            cx as i32 + regions[0] * 32,
                            cz as i32 + regions[1] * 32
                        );
                        self.add_chunk_column(cx, cz, column)
                    }
                    None => {}
                }
            }
        }
    }
    
    pub fn get_pending(&mut self, player: &Player) -> Option<ChunkBuffer<'a, R>> {
        use std::i32;
        // HACK(eddyb) find the closest chunk to the player.
        // The pending vector should be sorted instead.
        let pp = player.pos.map(|i| i as i32);
        let closest = self.pending_chunks.iter().enumerate().fold(
            (None, i32::max_value()),
            |(best_i, best_dist), (i, ref chunk_buf)| {
                let cc = chunk_buf.coords;
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
            match self.pending_chunks.len() {
                0 => None,
                _ => Some(self.pending_chunks.swap_remove(i))
            }
        });
        
        pending
    }

    pub fn each_chunk_and_neighbors<F>(&'a self, mut f: F)
        where F: FnMut(/*coords:*/ [i32; 3],
                       /*buffer:*/ &'a Option<gfx::handle::Buffer<R, Vertex>>,
                       /*chunks:*/ [[[&'a Chunk; 3]; 3]; 3],
                       /*biomes:*/ [[Option<&'a [[BiomeId; SIZE]; SIZE]>; 3]; 3]) {
                           
        for &(x, z) in self.chunk_columns.keys() {
            let columns = [-1, 0, 1].map(
                    |dz| [-1, 0, 1].map(
                        |dx| self.chunk_columns.get(&(x + dx, z + dz))
                    )
                );
            let central = columns[1][1].unwrap();
            for y in 0..central.chunks.len() {
                let chunks = [-1, 0, 1].map(|dy| {
                    let y = y as i32 + dy;
                    columns.map(
                        |cz| cz.map(
                            |cx| cx.and_then(
                                |c| c.chunks[..].get(y as usize)
                            ).unwrap_or(EMPTY_CHUNK)
                        )
                    )
                });
                f([x, y as i32, z], &mut central.buffers[y].borrow_mut(), chunks,
                  columns.map(|cz| cz.map(|cx| cx.map(|c| &c.biomes))))
            }
        }
    }

    pub fn each_chunk<F>(&self, mut f: F)
        where F: FnMut(/*x:*/ i32, /*y:*/ i32, /*z:*/ i32, /*c:*/ &Chunk, 
            /*b:*/ &RefCell<Option<gfx::handle::Buffer<R, Vertex>>>)
    {
        for (&(x, z), c) in self.chunk_columns.iter() {
            for (y, (c, b)) in c.chunks.iter()
                .zip(c.buffers.iter()).enumerate() {

                f(x, y as i32, z, c, b)
            }
        }
    }
}
