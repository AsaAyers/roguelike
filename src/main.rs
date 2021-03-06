#![allow(clippy::ptr_arg)]
extern crate rand;
extern crate tcod;

use rand::Rng;
use std::cmp;
use tcod::colors;
use tcod::colors::Color;
use tcod::console::*;
use tcod::input::Key;
use tcod::input::KeyCode::*;
use tcod::input::{self, Event, Mouse};
use tcod::map::{FovAlgorithm, Map as FovMap};
use PlayerAction::*;

const BAR_WIDTH: i32 = 20;
const PANEL_HEIGHT: i32 = 7;
const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;

const MSG_X: i32 = BAR_WIDTH + 2;
const MSG_WIDTH: i32 = SCREEN_WIDTH - BAR_WIDTH - 2;
const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;

const FOV_ALGO: FovAlgorithm = FovAlgorithm::Basic;
const FOV_LIGHT_WALLS: bool = true;
const TORCH_RADIUS: i32 = 10;

const FIREBALL_RADIUS: i32 = 3;
const FIREBALL_DAMAGE: i32 = 12;

const CONFUSE_RANGE: i32 = 8;
const CONFUSE_NUM_TURNS: i32 = 10;
const LIGHTNING_RANGE: i32 = 5;
const LIGHTNING_DAMAGE: i32 = 20;
const INVENTORY_WIDTH: i32 = 50;
const HEAL_AMOUNT: i32 = 4;
const PLAYER: usize = 0;
const MAX_ROOM_MONSTERS: i32 = 3;
const MAX_ROOM_ITEMS: i32 = 2;
const SCREEN_WIDTH: i32 = 80;
const SCREEN_HEIGHT: i32 = 50;
const LIMIT_FPS: i32 = 20;
const MAP_WIDTH: i32 = 80;
const MAP_HEIGHT: i32 = 43;
const ROOM_MAX_SIZE: i32 = 10;
const ROOM_MIN_SIZE: i32 = 6;
const MAX_ROOMS: i32 = 30;

const COLOR_DARK_WALL: Color = Color { r: 0, g: 0, b: 100 };
const COLOR_LIGHT_WALL: Color = Color {
    r: 130,
    g: 110,
    b: 50,
};
const COLOR_DARK_GROUND: Color = Color {
    r: 50,
    g: 50,
    b: 150,
};
const COLOR_LIGHT_GROUND: Color = Color {
    r: 200,
    g: 180,
    b: 50,
};

struct Tcod {
    root: Root,
    con: Offscreen,
    panel: Offscreen,
    fov: FovMap,
    mouse: Mouse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}

// combat-related properties and methods (monster, player, NPC).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fighter {
    max_hp: i32,
    hp: i32,
    defense: i32,
    power: i32,
    on_death: DeathCallback,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DeathCallback {
    Player,
    Monster,
}

impl DeathCallback {
    fn callback(self, messages: &mut Messages, object: &mut Object) {
        use DeathCallback::*;
        let callback: fn(&mut Messages, &mut Object) = match self {
            Player => player_death,
            Monster => monster_death,
        };
        callback(messages, object)
    }
}

fn player_death(messages: &mut Messages, player: &mut Object) {
    message(messages, "You died!", colors::RED);

    player.char = '%';
    player.color = colors::DARK_RED;
}

fn monster_death(messages: &mut Messages, monster: &mut Object) {
    // transform it into a nasty corpse! it doesn't block, can't be attacked and doesn't move.
    message(
        messages,
        format!("{} is dead!", monster.name),
        colors::ORANGE,
    );
    monster.char = '%';
    monster.color = colors::DARK_RED;
    monster.blocks = false;
    monster.fighter = None;
    monster.ai = None;
    monster.name = format!("remains of {}", monster.name)
}

#[derive(Clone, Debug, PartialEq)]
enum Ai {
    Basic,
    Confused {
        previous_ai: Box<Ai>,
        num_turns: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Heal,
    Lightning,
    Confuse,
    Fireball,
}

enum UseResult {
    UsedUp,
    Cancelled,
}

#[derive(Debug)]
struct Object {
    x: i32,
    y: i32,
    char: char,
    color: Color,
    name: String,
    blocks: bool,
    alive: bool,
    fighter: Option<Fighter>,
    ai: Option<Ai>,
    item: Option<Item>,
}

impl Object {
    pub fn new(x: i32, y: i32, char: char, name: &str, color: Color, blocks: bool) -> Self {
        Object {
            x,
            y,
            char,
            name: name.into(),
            color,
            blocks,
            alive: false,
            fighter: None,
            ai: None,
            item: None,
        }
    }

    /// set the color and then draw the character that represents this object at its position
    pub fn draw(&self, con: &mut Console) {
        con.set_default_foreground(self.color);
        con.put_char(self.x, self.y, self.char, BackgroundFlag::None);
    }

    /// Erase the character that represents this object
    pub fn clear(&self, con: &mut Console) {
        con.put_char(self.x, self.y, ' ', BackgroundFlag::None);
    }

    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn set_pos(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    pub fn distance(&self, x: i32, y: i32) -> f32 {
        let dx = x - self.x;
        let dy = y - self.y;
        ((dx.pow(2) + dy.pow(2)) as f32).sqrt()
    }
    pub fn distance_to(&self, other: &Object) -> f32 {
        self.distance(other.x, other.y)
    }

    pub fn take_damage(&mut self, messages: &mut Messages, damage: i32) {
        // apply damage if possible
        if let Some(fighter) = self.fighter.as_mut() {
            if damage > 0 {
                fighter.hp -= damage;
            }
        }
        if let Some(fighter) = self.fighter {
            if fighter.hp <= 0 {
                self.alive = false;
                fighter.on_death.callback(messages, self)
            }
        }
    }

    pub fn heal(&mut self, amount: i32) {
        if let Some(ref mut fighter) = self.fighter {
            fighter.hp += amount;
            if fighter.hp > fighter.max_hp {
                fighter.hp = fighter.max_hp
            }
        }
    }

    pub fn attack(&mut self, messages: &mut Messages, target: &mut Object) {
        // a simple formula for attack damage
        let damage = self.fighter.map_or(0, |f| f.power) - target.fighter.map_or(0, |f| f.defense);
        if damage > 0 {
            // make the target take some damage
            message(
                messages,
                format!(
                    "{} attacks {} for {} hit points.",
                    self.name, target.name, damage
                ),
                colors::RED,
            );
            target.take_damage(messages, damage);
        } else {
            message(
                messages,
                format!(
                    "{} attacks {} but it has no effect!",
                    self.name, target.name
                ),
                colors::GREEN,
            );
        }
    }
}

fn is_blocked(x: i32, y: i32, map: &Map, objects: &[Object]) -> bool {
    if map[x as usize][y as usize].blocked {
        return true;
    }

    objects
        .iter()
        .any(|object| object.blocks && object.pos() == (x, y))
}

fn move_by(id: usize, dx: i32, dy: i32, map: &Map, objects: &mut [Object]) {
    let (x, y) = objects[id].pos();
    if !is_blocked(x + dx, y + dy, map, objects) {
        objects[id].set_pos(x + dx, y + dy);
    }
}

fn move_towards(id: usize, target_x: i32, target_y: i32, map: &Map, objects: &mut [Object]) {
    // vector from this object to the target, and distance
    let dx = target_x - objects[id].x;
    let dy = target_y - objects[id].y;
    let distance = ((dx.pow(2) + dy.pow(2)) as f32).sqrt();

    // normalize it to length 1 (preserving direction), then round it and
    // convert to integer so the movement is restricted to the map grid
    let dx = (dx as f32 / distance).round() as i32;
    let dy = (dy as f32 / distance).round() as i32;
    move_by(id, dx, dy, map, objects);
}

fn pick_item_up(
    object_id: usize,
    objects: &mut Vec<Object>,
    inventory: &mut Vec<Object>,
    messages: &mut Messages,
) {
    if inventory.len() >= 26 {
        message(
            messages,
            format!(
                "Your inventory is full, cannot pick up {}.",
                objects[object_id].name
            ),
            colors::RED,
        )
    } else {
        let item = objects.swap_remove(object_id);
        message(
            messages,
            format!("You picked up a {}!", item.name),
            colors::GREEN,
        );
        inventory.push(item)
    }
}

fn player_move_or_attack(
    dx: i32,
    dy: i32,
    map: &Map,
    messages: &mut Messages,
    objects: &mut [Object],
) {
    // the coordinates the player is moving to/attacking
    let x = objects[PLAYER].x + dx;
    let y = objects[PLAYER].y + dy;

    // try to find an attackable object there
    let target_id = objects
        .iter()
        .position(|object| object.fighter.is_some() && object.pos() == (x, y));

    match target_id {
        Some(target_id) => {
            let (player, target) = mut_two(PLAYER, target_id, objects);
            player.attack(messages, target);
        }
        None => move_by(PLAYER, dx, dy, map, objects),
    }
}

fn cast_fireball(
    _inventory_id: usize,
    objects: &mut [Object],
    messages: &mut Messages,
    map: &mut Map,
    tcod: &mut Tcod,
) -> UseResult {
    // ask the player for a target tile to throw a fireball at
    message(
        messages,
        "Left-click a target tile for the fireball, or right-click to cancel.",
        colors::LIGHT_CYAN,
    );
    let (x, y) = match tcod.target_tile(objects, map, messages, None) {
        Some(tile_pos) => tile_pos,
        None => return UseResult::Cancelled,
    };
    message(
        messages,
        format!(
            "The fireball explodes, burning everything within {} tiles!",
            FIREBALL_RADIUS
        ),
        colors::ORANGE,
    );

    for obj in objects {
        if obj.distance(x, y) <= FIREBALL_RADIUS as f32 && obj.fighter.is_some() {
            message(
                messages,
                format!(
                    "The {} gets burned for {} hit points.",
                    obj.name, FIREBALL_DAMAGE
                ),
                colors::ORANGE,
            );
            obj.take_damage(messages, FIREBALL_DAMAGE);
        }
    }

    UseResult::UsedUp
}

fn cast_confuse(
    _inventory_id: usize,
    objects: &mut [Object],
    messages: &mut Messages,
    map: &mut Map,
    tcod: &mut Tcod,
) -> UseResult {
    message(
        messages,
        "Left-click an enemy to confuse it, or right-click to cancel.",
        colors::LIGHT_CYAN,
    );
    let monster_id = tcod.target_monster(objects, map, messages, Some(CONFUSE_RANGE as f32));
    if let Some(monster_id) = monster_id {
        let old_ai = objects[monster_id].ai.take().unwrap_or(Ai::Basic);
        // replace the monster's AI with a "confused" one; after
        // some turns it will restore the old AI
        objects[monster_id].ai = Some(Ai::Confused {
            previous_ai: Box::new(old_ai),
            num_turns: CONFUSE_NUM_TURNS,
        });
        message(
            messages,
            format!(
                "The eyes of {} look vacant, as he starts to stumble around!",
                objects[monster_id].name
            ),
            colors::LIGHT_GREEN,
        );
        UseResult::UsedUp
    } else {
        // no enemy fonud within maximum range
        message(messages, "No enemy is close enough to strike.", colors::RED);
        UseResult::Cancelled
    }
}

fn cast_lightning(
    _inventory_id: usize,
    objects: &mut [Object],
    messages: &mut Messages,
    map: &mut Map,
    tcod: &mut Tcod,
) -> UseResult {
    // find closest enemy (inside a maximum range) and damage it
    let monster_id = closest_monster(LIGHTNING_RANGE, objects, tcod);
    if let Some(monster_id) = monster_id {
        // zap it!
        message(
            messages,
            format!(
                "A lightning bolt strikes the {} with a loud thunder! \
                 The damage is {} hit points.",
                objects[monster_id].name, LIGHTNING_DAMAGE
            ),
            colors::LIGHT_BLUE,
        );
        objects[monster_id].take_damage(messages, LIGHTNING_DAMAGE);
        UseResult::UsedUp
    } else {
        // no enemy found within maximum range
        message(messages, "No enemy is close enough to strike.", colors::RED);
        UseResult::Cancelled
    }
}

fn closest_monster(max_range: i32, objects: &mut [Object], tcod: &Tcod) -> Option<usize> {
    let mut closest_enemy = None;
    let mut closest_dist = (max_range + 1) as f32;

    for (id, object) in objects.iter().enumerate() {
        if (id != PLAYER)
            && object.fighter.is_some()
            && object.ai.is_some()
            && tcod.fov.is_in_fov(object.x, object.y)
        {
            let dist = objects[PLAYER].distance_to(object);
            if dist < closest_dist {
                closest_enemy = Some(id);
                closest_dist = dist;
            }
        }
    }
    closest_enemy
}

fn cast_heal(
    _inventory_id: usize,
    objects: &mut [Object],
    messages: &mut Messages,
    map: &mut Map,
    _tcod: &mut Tcod,
) -> UseResult {
    if let Some(fighter) = objects[PLAYER].fighter {
        if fighter.hp == fighter.max_hp {
            message(messages, "You are already at full health.", colors::RED);
            return UseResult::Cancelled;
        }

        message(
            messages,
            "Your wounds start ot feel better!",
            colors::LIGHT_VIOLET,
        );
        objects[PLAYER].heal(HEAL_AMOUNT);
        return UseResult::UsedUp;
    }
    UseResult::Cancelled
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    blocked: bool,
    block_sight: bool,
    explored: bool,
}

impl Tile {
    pub fn empty() -> Self {
        Tile {
            blocked: false,
            block_sight: false,
            explored: false,
        }
    }

    pub fn wall() -> Self {
        Tile {
            blocked: true,
            block_sight: true,
            explored: false,
        }
    }
}

type Messages = Vec<(String, Color)>;
type Map = Vec<Vec<Tile>>;

fn message<T: Into<String>>(messages: &mut Messages, message: T, color: Color) {
    if messages.len() == MSG_HEIGHT {
        messages.remove(0);
    }

    messages.push((message.into(), color))
}

fn make_map(objects: &mut Vec<Object>) -> Map {
    // fill map with "unblocked" tiles
    let mut map = vec![vec![Tile::wall(); MAP_HEIGHT as usize]; MAP_WIDTH as usize];
    let mut starting_position = (0, 0);

    let mut rooms = vec![];

    for _ in 0..MAX_ROOMS {
        // random width and height
        let w = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        let h = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        // random position without going out of the boundaries of the map
        let x = rand::thread_rng().gen_range(0, MAP_WIDTH - w);
        let y = rand::thread_rng().gen_range(0, MAP_HEIGHT - h);
        let new_room = Rect::new(x, y, w, h);

        // run through the other rooms and see if they intersect with this one
        let failed = rooms
            .iter()
            .any(|other_room| new_room.intersects_with(other_room));

        if !failed {
            // this means there are no intersections, so this room is valid

            // "paint" it to the map's tiles
            create_room(new_room, &mut map);
            place_objects(new_room, &map, objects);

            // center coordinates of the new room, will be useful later
            let (new_x, new_y) = new_room.center();

            if rooms.is_empty() {
                // this is the first room, where the player starts at
                starting_position = (new_x, new_y);
            } else {
                // all rooms after the first:
                // connect it to the previous room with a tunnel

                // center coordinates of the previous room
                let (prev_x, prev_y) = rooms[rooms.len() - 1].center();

                // draw a coin (random bool value -- either true or false)
                if rand::random() {
                    // first move horizontally, then vertically
                    create_h_tunnel(prev_x, new_x, prev_y, &mut map);
                    create_v_tunnel(prev_y, new_y, new_x, &mut map);
                } else {
                    // first move vertically, then horizontally
                    create_v_tunnel(prev_y, new_y, prev_x, &mut map);
                    create_h_tunnel(prev_x, new_x, new_y, &mut map);
                }
            }
            // finally, append the new room to the list
            rooms.push(new_room);
        }
    }

    objects[PLAYER].set_pos(starting_position.0, starting_position.1);
    map
}

fn place_objects(room: Rect, map: &Map, objects: &mut Vec<Object>) {
    let num_monsters = rand::thread_rng().gen_range(0, MAX_ROOM_MONSTERS + 1);
    let num_items = rand::thread_rng().gen_range(0, MAX_ROOM_ITEMS + 1);

    for _ in 0..num_monsters {
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        if !is_blocked(x, y, map, objects) {
            let mut monster = if rand::random::<f32>() < 0.8 {
                // create an orc
                let mut orc = Object::new(x, y, 'o', "Orc", colors::DESATURATED_GREEN, true);
                orc.fighter = Some(Fighter {
                    max_hp: 10,
                    hp: 10,
                    defense: 0,
                    power: 3,
                    on_death: DeathCallback::Monster,
                });
                orc.ai = Some(Ai::Basic);
                orc
            } else {
                let mut troll = Object::new(x, y, 'T', "Troll", colors::DARKER_GREEN, true);
                troll.fighter = Some(Fighter {
                    max_hp: 16,
                    hp: 16,
                    defense: 1,
                    power: 4,
                    on_death: DeathCallback::Monster,
                });
                troll.ai = Some(Ai::Basic);
                troll
            };
            monster.alive = true;

            objects.push(monster);
        }
    }

    for _ in 0..num_items {
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        if !is_blocked(x, y, map, objects) {
            let dice = rand::random::<f32>();

            let item = if dice < 0.7 {
                let mut object = Object::new(x, y, '!', "Healing Potion", colors::VIOLET, false);
                object.item = Some(Item::Heal);
                object
            } else if dice < 0.7 + 0.1 {
                let mut object = Object::new(
                    x,
                    y,
                    '#',
                    "Scroll of lightning bolt",
                    colors::LIGHT_YELLOW,
                    false,
                );
                object.item = Some(Item::Lightning);
                object
            } else if dice < 0.7 + 0.1 + 0.1 {
                // create a fireball scroll (10% chance)
                let mut object =
                    Object::new(x, y, '#', "scroll of fireball", colors::LIGHT_YELLOW, false);
                object.item = Some(Item::Fireball);
                object
            } else {
                // create a confuse scroll (15% chance)
                let mut object = Object::new(
                    x,
                    y,
                    '#',
                    "scroll of confusion",
                    colors::LIGHT_YELLOW,
                    false,
                );
                object.item = Some(Item::Confuse);
                object
            };

            objects.push(item);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect {
            x1: x,
            y1: y,
            x2: x + w,
            y2: y + h,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        let center_x = (self.x1 + self.x2) / 2;
        let center_y = (self.y1 + self.y2) / 2;
        (center_x, center_y)
    }

    pub fn intersects_with(&self, other: &Rect) -> bool {
        // returns true if this rectangle intersects with another one
        (self.x1 <= other.x2)
            && (self.x2 >= other.x1)
            && (self.y1 <= other.y2)
            && (self.y2 >= other.y1)
    }
}

fn create_room(room: Rect, map: &mut Map) {
    for x in (room.x1 + 1)..room.x2 {
        for y in (room.y1 + 1)..room.y2 {
            map[x as usize][y as usize] = Tile::empty();
        }
    }
}

fn create_h_tunnel(x1: i32, x2: i32, y: i32, map: &mut Map) {
    for x in cmp::min(x1, x2)..=cmp::max(x1, x2) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_v_tunnel(y1: i32, y2: i32, x: i32, map: &mut Map) {
    for y in cmp::min(y1, y2)..=cmp::max(y1, y2) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

/// Mutably borrow two *separate* elements from the given slice.
/// Panics when the indexes are equal or out of bounds.
fn mut_two<T>(first_index: usize, second_index: usize, items: &mut [T]) -> (&mut T, &mut T) {
    assert!(first_index != second_index);
    let split_at_index = cmp::max(first_index, second_index);
    let (first_slice, second_slice) = items.split_at_mut(split_at_index);
    if first_index < second_index {
        (&mut first_slice[first_index], &mut second_slice[0])
    } else {
        (&mut second_slice[0], &mut first_slice[second_index])
    }
}

fn ai_take_turn(
    monster_id: usize,
    map: &Map,
    messages: &mut Messages,
    objects: &mut [Object],
    tcod: &Tcod,
) {
    use Ai::*;
    if let Some(ai) = objects[monster_id].ai.take() {
        let new_ai = match ai {
            Basic => ai_basic(monster_id, map, objects, tcod, messages),
            Confused {
                previous_ai,
                num_turns,
            } => ai_confused(monster_id, map, objects, messages, previous_ai, num_turns),
        };
        objects[monster_id].ai = Some(new_ai)
    }
}

fn ai_confused(
    monster_id: usize,
    map: &Map,
    objects: &mut [Object],
    messages: &mut Messages,
    previous_ai: Box<Ai>,
    num_turns: i32,
) -> Ai {
    if num_turns > 0 {
        // move in a random idrection, and decrease the number of turns confused
        move_by(
            monster_id,
            rand::thread_rng().gen_range(-1, 2),
            rand::thread_rng().gen_range(-1, 2),
            map,
            objects,
        );
        Ai::Confused {
            previous_ai,
            num_turns: num_turns - 1,
        }
    } else {
        message(
            messages,
            format!("The {} is no longer confused!", objects[monster_id].name),
            colors::RED,
        );

        *previous_ai
    }
}

fn ai_basic(
    monster_id: usize,
    map: &Map,
    objects: &mut [Object],
    tcod: &Tcod,
    messages: &mut Messages,
) -> Ai {
    // a basic monster takes its turn. If you can see it, it can see you
    let (monster_x, monster_y) = objects[monster_id].pos();
    if tcod.fov.is_in_fov(monster_x, monster_y) {
        if objects[monster_id].distance_to(&objects[PLAYER]) >= 2.0 {
            // move towards player if far away
            let (player_x, player_y) = objects[PLAYER].pos();
            move_towards(monster_id, player_x, player_y, map, objects);
        } else if objects[PLAYER].fighter.map_or(false, |f| f.hp > 0) {
            // close enough, attack! (if the player is still alive.)
            let (monster, player) = mut_two(monster_id, PLAYER, objects);
            monster.attack(messages, player);
        }
    }
    Ai::Basic
}

fn main() {
    let mut messages = vec![];

    let mut tcod = Tcod {
        root: Root::initializer()
            .font("arial10x10.png", FontLayout::Tcod)
            .font_type(FontType::Greyscale)
            .size(SCREEN_WIDTH, SCREEN_HEIGHT)
            .title("Rust/libtcod tutorial")
            .init(),
        con: Offscreen::new(MAP_WIDTH, MAP_HEIGHT),
        panel: Offscreen::new(SCREEN_WIDTH, PANEL_HEIGHT),
        fov: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        mouse: Default::default(),
    };

    message(
        &mut messages,
        "Welcome stranger! Prepare to perish in the Tombs of the Ancient Kings.",
        colors::RED,
    );

    tcod::system::set_fps(LIMIT_FPS);

    let mut player = Object::new(0, 0, '@', "player", colors::WHITE, true);
    player.alive = true;
    player.fighter = Some(Fighter {
        max_hp: 30,
        hp: 30,
        defense: 2,
        power: 5,
        on_death: DeathCallback::Player,
    });
    let mut inventory: Vec<Object> = vec![];
    let mut objects = vec![player];
    let mut map = make_map(&mut objects);
    let mut previous_player_position = (-1, -1);
    let mut key = Default::default();

    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            tcod.fov.set(
                x,
                y,
                !map[x as usize][y as usize].block_sight,
                !map[x as usize][y as usize].blocked,
            );
        }
    }

    while !tcod.root.window_closed() {
        tcod.con.set_default_foreground(colors::WHITE);
        tcod.root.clear();

        let fov_recompute = previous_player_position != (objects[0].x, objects[0].y);

        match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
            Some((_, Event::Mouse(m))) => tcod.mouse = m,
            Some((_, Event::Key(k))) => key = k,
            _ => key = Default::default(),
        };

        render_all(&mut tcod, &objects, &mut map, &messages, fov_recompute);
        tcod.root.flush();
        for object in &objects {
            object.clear(&mut tcod.con)
        }
        let player = &mut objects[0];

        previous_player_position = (player.x, player.y);
        let player_action = handle_keys(
            key,
            &mut tcod,
            &mut map,
            &mut objects,
            &mut inventory,
            &mut messages,
        );
        if player_action == PlayerAction::Exit {
            break;
        }

        if objects[PLAYER].alive && player_action != PlayerAction::DidntTakeTurn {
            for id in 0..objects.len() {
                if objects[id].ai.is_some() {
                    ai_take_turn(id, &map, &mut messages, &mut objects, &tcod);
                }
            }
        }
    }
}

fn get_names_under_mouse(tcod: &Tcod, objects: &[Object]) -> String {
    let (x, y) = (tcod.mouse.cx as i32, tcod.mouse.cy as i32);

    let names = objects
        .iter()
        .filter(|obj| obj.pos() == (x, y) && tcod.fov.is_in_fov(obj.x, obj.y))
        .map(|obj| obj.name.clone())
        .collect::<Vec<_>>();

    names.join(", ")
}

impl Tcod {
    pub fn target_tile(
        self: &mut Tcod,
        objects: &[Object],
        map: &mut Map,
        messages: &Messages,
        max_range: Option<f32>,
    ) -> Option<(i32, i32)> {
        use tcod::input::KeyCode::Escape;
        loop {
            self.root.flush();

            let event = input::check_for_event(input::KEY_PRESS | input::MOUSE).map(|e| e.1);
            let mut key = None;
            match event {
                Some(Event::Mouse(m)) => self.mouse = m,
                Some(Event::Key(k)) => key = Some(k),
                None => {}
            }
            render_all(self, objects, map, messages, false);

            let (x, y) = (self.mouse.cx as i32, self.mouse.cy as i32);

            // accept the target if the player clicked in FOV, and in case a range
            // is specified, if it's in that range
            let in_fov = (x < MAP_WIDTH) && (y < MAP_HEIGHT) && self.fov.is_in_fov(x, y);
            let in_range = max_range.map_or(true, |range| objects[PLAYER].distance(x, y) <= range);
            if self.mouse.lbutton_pressed && in_fov && in_range {
                return Some((x, y));
            }

            let escape = key.map_or(false, |k| k.code == Escape);
            if self.mouse.rbutton_pressed || escape {
                return None; // cancel if the player right-clicked or pressed Escape
            }
        }
    }

    fn target_monster(
        self: &mut Tcod,
        objects: &[Object],
        map: &mut Map,
        messages: &Messages,
        max_range: Option<f32>,
    ) -> Option<usize> {
        loop {
            match self.target_tile(objects, map, messages, max_range) {
                Some((x, y)) => {
                    // return the first clicked monster, otherwise continue looping
                    for (id, obj) in objects.iter().enumerate() {
                        if obj.pos() == (x, y) && obj.fighter.is_some() && id != PLAYER {
                            return Some(id);
                        }
                    }
                }
                None => return None,
            }
        }
    }

    pub fn menu<T: AsRef<str>>(
        self: &mut Tcod,
        header: &str,
        options: &[T],
        width: i32,
    ) -> Option<usize> {
        assert!(
            options.len() <= 26,
            "Cannot have a menu with more than 26 options"
        );

        // calculate total height for the header (after auto-wrap) and one line per option
        let header_height = self
            .root
            .get_height_rect(0, 0, width, SCREEN_HEIGHT, header);
        let height = options.len() as i32 + header_height;

        // create an off-screen console that represents the menu's window
        let mut window = Offscreen::new(width, height);

        // print the header, with auto-wrap
        window.set_default_foreground(colors::WHITE);
        window.print_rect_ex(
            0,
            0,
            width,
            height,
            BackgroundFlag::None,
            TextAlignment::Left,
            header,
        );

        for (index, option_text) in options.iter().enumerate() {
            let menu_letter = (b'a' + index as u8) as char;
            let text = format!("({}) {}", menu_letter, option_text.as_ref());
            window.print_ex(
                0,
                header_height + index as i32,
                BackgroundFlag::None,
                TextAlignment::Left,
                text,
            );
        }

        // blit the contents of "window" to the root console
        let x = SCREEN_WIDTH / 2 - width / 2;
        let y = SCREEN_HEIGHT / 2 - height / 2;
        tcod::console::blit(
            &window,
            (0, 0),
            (width, height),
            &mut self.root,
            (x, y),
            1.0,
            0.7,
        );

        // present the root console to the player and wait for a key-press
        self.root.flush();
        let key = self.root.wait_for_keypress(true);

        // convert the ASCII code to an index; if it corresponds to an option, return it
        if key.printable.is_alphabetic() {
            let index = key.printable.to_ascii_lowercase() as usize - 'a' as usize;
            if index < options.len() {
                Some(index)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn render_bar(
        self: &mut Tcod,
        x: i32,
        y: i32,
        total_width: i32,
        name: &str,
        value: i32,
        maximum: i32,
        bar_color: Color,
        back_color: Color,
    ) {
        // render a bar (HP, experience, etc).
        let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

        //render the background first
        self.panel.set_default_background(back_color);
        self.panel
            .rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

        // now render the bar on top
        self.panel.set_default_background(bar_color);
        if bar_width > 0 {
            self.panel
                .rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
        }

        self.panel.set_default_foreground(colors::WHITE);
        self.panel.print_ex(
            x + total_width / 2,
            y,
            BackgroundFlag::None,
            TextAlignment::Center,
            &format!("{}: {}/{}", name, value, maximum),
        )
    }
    pub fn inventory_menu(self: &mut Tcod, inventory: &[Object], header: &str) -> Option<usize> {
        let options = if inventory.is_empty() {
            vec!["inventory is empty".into()]
        } else {
            inventory.iter().map(|item| item.name.clone()).collect()
        };

        let inventory_index = self.menu(header, &options, INVENTORY_WIDTH);

        if !inventory.is_empty() {
            inventory_index
        } else {
            None
        }
    }
}

fn use_item(
    inventory_id: usize,
    inventory: &mut Vec<Object>,
    objects: &mut [Object],
    messages: &mut Messages,
    map: &mut Map,
    tcod: &mut Tcod,
) {
    use Item::*;

    if let Some(item) = inventory[inventory_id].item {
        let on_use = match item {
            Heal => cast_heal,
            Lightning => cast_lightning,
            Confuse => cast_confuse,
            Fireball => cast_fireball,
        };

        match on_use(inventory_id, objects, messages, map, tcod) {
            UseResult::UsedUp => {
                inventory.remove(inventory_id);
            }
            UseResult::Cancelled => {
                message(messages, "Cancelled", colors::WHITE);
            }
        }
    } else {
        message(
            messages,
            format!("The {} cannot be used.", inventory[inventory_id].name),
            colors::WHITE,
        );
    }
}

fn handle_keys(
    key: Key,
    tcod: &mut Tcod,
    map: &mut Map,
    objects: &mut Vec<Object>,
    inventory: &mut Vec<Object>,
    messages: &mut Messages,
) -> PlayerAction {
    let player_alive = objects[PLAYER].alive;
    match (key, player_alive) {
        (Key { code: Up, .. }, true) => {
            player_move_or_attack(0, -1, &map, messages, objects);
            TookTurn
        }
        (Key { code: Down, .. }, true) => {
            player_move_or_attack(0, 1, &map, messages, objects);
            TookTurn
        }
        (Key { code: Left, .. }, true) => {
            player_move_or_attack(-1, 0, &map, messages, objects);
            TookTurn
        }
        (Key { code: Right, .. }, true) => {
            player_move_or_attack(1, 0, &map, messages, objects);
            TookTurn
        }
        (Key { printable: 'i', .. }, true) => {
            let inventory_index = tcod.inventory_menu(
                inventory,
                "Press the key next to an item to use it, or any other to cancel.\n",
            );
            if let Some(inventory_index) = inventory_index {
                use_item(inventory_index, inventory, objects, messages, map, tcod);
                TookTurn
            } else {
                DidntTakeTurn
            }
        }
        (Key { printable: 'g', .. }, true) => {
            let item_id = objects
                .iter()
                .position(|object| object.pos() == objects[PLAYER].pos() && object.item.is_some());
            if let Some(item_id) = item_id {
                pick_item_up(item_id, objects, inventory, messages);
            }
            DidntTakeTurn
        }
        (
            Key {
                code: Enter,
                alt: true,
                ..
            },
            _,
        ) => {
            let fullscreen = tcod.root.is_fullscreen();
            tcod.root.set_fullscreen(!fullscreen);
            DidntTakeTurn
        }
        (Key { code: Escape, .. }, _) => Exit,
        _ => DidntTakeTurn,
    }
}

fn render_all(
    tcod: &mut Tcod,
    objects: &[Object],
    map: &mut Map,
    messages: &Messages,
    fov_recompute: bool,
) {
    if fov_recompute {
        let player = &objects[0];
        tcod.fov
            .compute_fov(player.x, player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO)
    }
    let mut to_draw: Vec<_> = objects
        .iter()
        .filter(|o| tcod.fov.is_in_fov(o.x, o.y))
        .collect();

    to_draw.sort_by(|o1, o2| o1.blocks.cmp(&o2.blocks));

    for object in &to_draw {
        object.draw(&mut tcod.con);
    }

    // go through all tiles, and set their background color
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let visible = tcod.fov.is_in_fov(x, y);
            let wall = map[x as usize][y as usize].block_sight;
            let color = match (visible, wall) {
                // outside of field of view:
                (false, true) => COLOR_DARK_WALL,
                (false, false) => COLOR_DARK_GROUND,
                // inside fov:
                (true, true) => COLOR_LIGHT_WALL,
                (true, false) => COLOR_LIGHT_GROUND,
            };
            // con.set_char_background(x, y, color, BackgroundFlag::Set);

            let explored = &mut map[x as usize][y as usize].explored;
            if visible {
                // since it's visible, explore it
                *explored = true;
            }
            if *explored {
                // show explored tiles only (any visible tile is explored already)
                tcod.con
                    .set_char_background(x, y, color, BackgroundFlag::Set);
            }
        }
    }

    blit(
        &tcod.con,
        (0, 0),
        (MAP_WIDTH, MAP_HEIGHT),
        &mut tcod.root,
        (0, 0),
        1.0,
        1.0,
    );

    tcod.panel.set_default_background(colors::BLACK);
    tcod.panel.clear();

    // show the player's stats
    let hp = objects[PLAYER].fighter.map_or(0, |f| f.hp);
    let max_hp = objects[PLAYER].fighter.map_or(0, |f| f.max_hp);
    tcod.render_bar(
        1,
        1,
        BAR_WIDTH,
        "HP",
        hp,
        max_hp,
        colors::LIGHT_RED,
        colors::DARKER_RED,
    );

    tcod.panel.set_default_foreground(colors::LIGHT_GREY);
    tcod.panel.print_ex(
        1,
        0,
        BackgroundFlag::None,
        TextAlignment::Left,
        get_names_under_mouse(tcod, objects),
    );

    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in messages.iter().rev() {
        let msg_height = tcod.panel.get_height_rect(MSG_X, y, MSG_WIDTH, 0, msg);
        y -= msg_height;

        if y < 0 {
            break;
        }

        tcod.panel.set_default_foreground(color);
        tcod.panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg)
    }

    // blit the contents of `panel` to the root console
    blit(
        &tcod.panel,
        (0, 0),
        (SCREEN_WIDTH, PANEL_HEIGHT),
        &mut tcod.root,
        (0, PANEL_Y),
        1.0,
        1.0,
    );
}
