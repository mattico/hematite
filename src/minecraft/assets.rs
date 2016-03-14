use gfx;

use minecraft::biome::Biomes;
use minecraft::block_state::BlockStates;

pub struct Assets<R: gfx::Resources> {
	pub biomes: Biomes,
	pub block_states: BlockStates<R>,
}