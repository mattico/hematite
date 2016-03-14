use array::*;
use vecmath::*;

use minecraft::nbt::Nbt;

pub struct Player {
    pub pos: Vector3<f32>,
    pub yaw: f32,
    pub pitch: f32,
}

impl Player {
    pub fn from_nbt(world: &Nbt) -> Player {
        let player_pos = Array::from_iter(
                world["Data"]["Player"]["Pos"]
                    .as_double_list().unwrap().iter().map(|&x| x as f32)
            );
        let player_rot = world["Data"]["Player"]["Rotation"]
            .as_float_list().unwrap();
        let player_yaw = player_rot[0];
        let player_pitch = player_rot[1];

        Player {
            pos: player_pos,
            yaw: player_yaw,
            pitch: player_pitch,
        }
    }
}