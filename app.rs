// ============================================================================
// THE SMILING DARK: ASCENSION - COMPLETE EDITION
// A horror roguelike showcasing expert-level game architecture patterns
// ============================================================================

use macroquad::prelude::*;
use macroquad::ui::{hash, root_ui};
use serde::{Deserialize, Serialize};
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

// ============================================================================
// CONFIGURATION
// ============================================================================
const GRID_W: i32 = 16;
const GRID_H: i32 = 16;
const TILE_SIZE: f32 = 34.0;
const OFFSET_X: f32 = 260.0;
const OFFSET_Y: f32 = 60.0;
const FIXED_DT: f32 = 1.0 / 60.0;
const INPUT_BUFFER_WINDOW: f32 = 0.18;
const PARTICLE_POOL_SIZE: usize = 400;

// ============================================================================
// PATTERN #14: DETERMINISTIC RNG (xorshift64*)
// ============================================================================
struct DeterministicRng {
    state: u64,
}
impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xDEADBEEFCAFEBABE } else { seed },
        }
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        min + (self.next_u64() as i32 % (max - min + 1)).abs()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }
}

// ============================================================================
// PATTERN #1: SERVICE LOCATOR
// ============================================================================
struct Services {
    rng: Rc<RefCell<DeterministicRng>>,
    events: Rc<RefCell<EventBus>>,
    time: Rc<RefCell<TimeService>>,
    audio: Rc<RefCell<AudioService>>,
}
impl Services {
    fn new(seed: u64) -> Self {
        Self {
            rng: Rc::new(RefCell::new(DeterministicRng::new(seed))),
            events: Rc::new(RefCell::new(EventBus::new())),
            time: Rc::new(RefCell::new(TimeService::new(FIXED_DT))),
            audio: Rc::new(RefCell::new(AudioService::new())),
        }
    }
}

// ============================================================================
// PATTERN #8: FIXED TIMESTEP
// ============================================================================
struct TimeService {
    fixed_dt: f32,
    accumulator: f32,
    total: f32,
    frame_dt: f32,
}
impl TimeService {
    fn new(fixed_dt: f32) -> Self {
        Self {
            fixed_dt,
            accumulator: 0.0,
            total: 0.0,
            frame_dt: 0.0,
        }
    }
    fn update(&mut self, dt: f32) {
        self.frame_dt = dt;
        self.total += dt;
        self.accumulator += dt;
    }
    fn consume_step(&mut self) -> bool {
        if self.accumulator >= self.fixed_dt {
            self.accumulator -= self.fixed_dt;
            true
        } else {
            false
        }
    }
    fn alpha(&self) -> f32 {
        (self.accumulator / self.fixed_dt).clamp(0.0, 1.0)
    }
}

// ============================================================================
// PATTERN #2: EVENT BUS (Pub/Sub)
// ============================================================================
trait GameEvent: Any + Send + 'static {
    fn clone_box(&self) -> Box<dyn GameEvent>;
    fn as_any(&self) -> &dyn Any;
}
impl<T: Any + Clone + Send + 'static> GameEvent for T {
    fn clone_box(&self) -> Box<dyn GameEvent> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct EventBus {
    handlers: HashMap<TypeId, Vec<Rc<dyn Fn(&dyn Any)>>>,
    queue: VecDeque<Box<dyn GameEvent>>,
}
impl EventBus {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            queue: VecDeque::new(),
        }
    }
    fn subscribe<E: Clone + Send + 'static>(&mut self, f: Rc<dyn Fn(&E)>) {
        let wrapped: Rc<dyn Fn(&dyn Any)> = Rc::new(move |any| {
            if let Some(e) = any.downcast_ref::<E>() {
                f(e);
            }
        });
        self.handlers
            .entry(TypeId::of::<E>())
            .or_insert_with(Vec::new)
            .push(wrapped);
    }
    fn publish<E: Clone + Send + 'static>(&mut self, e: E) {
        self.queue.push_back(Box::new(e));
    }
    fn dispatch(&mut self) {
        while let Some(ev) = self.queue.pop_front() {
            let tid = ev.as_any().type_id();
            if let Some(handlers) = self.handlers.get(&tid) {
                for h in handlers {
                    h(ev.as_any());
                }
            }
        }
    }
}

// Game events
#[derive(Clone)]
struct EvPlayerMoved {
    from: GridPos,
    to: GridPos,
}
#[derive(Clone)]
struct EvItemCollected {
    kind: ItemKind,
    pos: GridPos,
}
#[derive(Clone)]
struct EvDamage {
    amount: f32,
    source: String,
}
#[derive(Clone)]
struct EvAchievement {
    name: String,
}
#[derive(Clone)]
struct EvFloorChanged {
    floor: i32,
}
#[derive(Clone)]
struct EvEnemySpotted {
    dist: f32,
}
#[derive(Clone)]
struct EvWallPlaced {
    pos: GridPos,
}
#[derive(Clone)]
struct EvCombo {
    count: i32,
}

// ============================================================================
// PATTERN #13: INPUT BUFFERING
// ============================================================================
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InputAction {
    Up,
    Down,
    Left,
    Right,
    Wall,
    Pause,
    Interact,
}

struct InputBuffer {
    buf: VecDeque<(InputAction, f32)>,
    window: f32,
}
impl InputBuffer {
    fn new(window: f32) -> Self {
        Self {
            buf: VecDeque::new(),
            window,
        }
    }
    fn push(&mut self, a: InputAction, t: f32) {
        self.buf.push_back((a, t));
        self.prune(t);
    }
    fn consume(&mut self, a: InputAction, t: f32) -> bool {
        if let Some(idx) = self
            .buf
            .iter()
            .position(|(x, ts)| *x == a && t - *ts <= self.window)
        {
            self.buf.remove(idx);
            true
        } else {
            false
        }
    }
    fn has(&self, a: InputAction, t: f32) -> bool {
        self.buf
            .iter()
            .any(|(x, ts)| *x == a && t - *ts <= self.window)
    }
    fn prune(&mut self, t: f32) {
        self.buf.retain(|(_, ts)| t - *ts <= self.window);
    }
    fn combo(&self, seq: &[InputAction], t: f32) -> bool {
        let recent: Vec<_> = self
            .buf
            .iter()
            .filter(|(_, ts)| t - *ts <= self.window)
            .map(|(a, _)| *a)
            .collect();
        if recent.len() < seq.len() {
            return false;
        }
        recent.windows(seq.len()).any(|w| w == seq)
    }
}

// ============================================================================
// PATTERN #5: OBJECT POOL (Particles)
// ============================================================================
#[derive(Clone)]
struct Particle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    emoji: String,
    color: Color,
    size: f32,
    gravity: f32,
    active: bool,
}
impl Particle {
    fn new() -> Self {
        Self {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            life: 0.0,
            max_life: 1.0,
            emoji: "✨".into(),
            color: WHITE,
            size: 20.0,
            gravity: 0.0,
            active: false,
        }
    }
    fn reset(&mut self) {
        self.pos = Vec2::ZERO;
        self.vel = Vec2::ZERO;
        self.life = 0.0;
        self.active = false;
    }
}

struct ParticlePool {
    items: Vec<Particle>,
}
impl ParticlePool {
    fn new(cap: usize) -> Self {
        Self {
            items: (0..cap).map(|_| Particle::new()).collect(),
        }
    }
    fn spawn(
        &mut self,
        pos: Vec2,
        vel: Vec2,
        emoji: &str,
        color: Color,
        life: f32,
        size: f32,
        gravity: f32,
    ) {
        for p in &mut self.items {
            if !p.active {
                p.pos = pos;
                p.vel = vel;
                p.emoji = emoji.into();
                p.color = color;
                p.life = life;
                p.max_life = life;
                p.size = size;
                p.gravity = gravity;
                p.active = true;
                return;
            }
        }
    }
    fn update(&mut self, dt: f32) {
        for p in &mut self.items {
            if !p.active {
                continue;
            }
            p.vel.y += p.gravity * dt;
            p.pos.x += p.vel.x * dt;
            p.pos.y += p.vel.y * dt;
            p.life -= dt;
            if p.life <= 0.0 {
                p.reset();
            }
        }
    }
    fn render(&self, font: &Font) {
        for p in &self.items {
            if !p.active {
                continue;
            }
            let a = (p.life / p.max_life).clamp(0.0, 1.0);
            draw_text_ex(
                &p.emoji,
                p.pos.x,
                p.pos.y,
                TextParams {
                    font: Some(font.clone()),
                    font_size: (p.size * a) as u16,
                    color: Color::new(p.color.r, p.color.g, p.color.b, a),
                    ..Default::default()
                },
            );
        }
    }
}

// ============================================================================
// PATTERN #6: SPATIAL HASH
// ============================================================================
struct SpatialHash {
    cell: i32,
    buckets: HashMap<(i32, i32), Vec<GridPos>>,
}
impl SpatialHash {
    fn new(cell: i32) -> Self {
        Self {
            cell,
            buckets: HashMap::new(),
        }
    }
    fn clear(&mut self) {
        self.buckets.clear();
    }
    fn insert(&mut self, p: GridPos) {
        let k = (p.x / self.cell, p.y / self.cell);
        self.buckets.entry(k).or_default().push(p);
    }
    fn query_radius(&self, center: GridPos, r: i32) -> Vec<GridPos> {
        let mut out = Vec::new();
        let cr = (r / self.cell) + 1;
        let cx = center.x / self.cell;
        let cy = center.y / self.cell;
        for dy in -cr..=cr {
            for dx in -cr..=cr {
                if let Some(v) = self.buckets.get(&(cx + dx, cy + dy)) {
                    for &p in v {
                        if p.manhattan(center) <= r {
                            out.push(p);
                        }
                    }
                }
            }
        }
        out
    }
}

// ============================================================================
// PATTERN #4: COMMAND PATTERN
// ============================================================================
trait Command {
    fn execute(&self, world: &mut World);
    fn name(&self) -> &str;
}
struct CmdMove {
    dir: Dir,
}
impl Command for CmdMove {
    fn execute(&self, w: &mut World) {
        w.try_move_player(self.dir);
    }
    fn name(&self) -> &str {
        "Move"
    }
}
struct CmdPlaceWall {
    pos: GridPos,
}
impl Command for CmdPlaceWall {
    fn execute(&self, w: &mut World) {
        w.place_wall(self.pos);
    }
    fn name(&self) -> &str {
        "PlaceWall"
    }
}
struct CmdUseItem {
    kind: ItemKind,
}
impl Command for CmdUseItem {
    fn execute(&self, w: &mut World) {
        w.use_item(self.kind);
    }
    fn name(&self) -> &str {
        "UseItem"
    }
}

struct CommandQueue {
    pending: VecDeque<Box<dyn Command>>,
}
impl CommandQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
    fn push(&mut self, c: Box<dyn Command>) {
        self.pending.push_back(c);
    }
    fn drain(&mut self, world: &mut World) {
        while let Some(c) = self.pending.pop_front() {
            c.execute(world);
        }
    }
}

// ============================================================================
// PATTERN #7: BEHAVIOR TREES (Enemy AI)
// ============================================================================
#[derive(Clone, Copy, PartialEq)]
enum BTStatus {
    Success,
    Failure,
    Running,
}
trait BTNode: Send {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus;
    fn reset(&mut self);
}

struct BTSeq {
    children: Vec<Box<dyn BTNode>>,
    idx: usize,
}
impl BTSeq {
    fn new(c: Vec<Box<dyn BTNode>>) -> Self {
        Self { children: c, idx: 0 }
    }
}
impl BTNode for BTSeq {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        while self.idx < self.children.len() {
            match self.children[self.idx].tick(ctx) {
                BTStatus::Failure => {
                    self.reset();
                    return BTStatus::Failure;
                }
                BTStatus::Running => return BTStatus::Running,
                BTStatus::Success => self.idx += 1,
            }
        }
        self.reset();
        BTStatus::Success
    }
    fn reset(&mut self) {
        self.idx = 0;
        for c in &mut self.children {
            c.reset();
        }
    }
}

struct BTSel {
    children: Vec<Box<dyn BTNode>>,
    idx: usize,
}
impl BTSel {
    fn new(c: Vec<Box<dyn BTNode>>) -> Self {
        Self { children: c, idx: 0 }
    }
}
impl BTNode for BTSel {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        while self.idx < self.children.len() {
            match self.children[self.idx].tick(ctx) {
                BTStatus::Success => {
                    self.reset();
                    return BTStatus::Success;
                }
                BTStatus::Running => return BTStatus::Running,
                BTStatus::Failure => self.idx += 1,
            }
        }
        self.reset();
        BTStatus::Failure
    }
    fn reset(&mut self) {
        self.idx = 0;
        for c in &mut self.children {
            c.reset();
        }
    }
}

struct BTCheckLOS;
impl BTNode for BTCheckLOS {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        if ctx.los {
            BTStatus::Success
        } else {
            BTStatus::Failure
        }
    }
    fn reset(&mut self) {}
}

struct BTCheckClose {
    threshold: i32,
}
impl BTNode for BTCheckClose {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        if ctx.dist <= self.threshold {
            BTStatus::Success
        } else {
            BTStatus::Failure
        }
    }
    fn reset(&mut self) {}
}

struct BTMoveToward;
impl BTNode for BTMoveToward {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        ctx.desired_move = Some(Dir::toward(ctx.self_pos, ctx.target_pos));
        BTStatus::Success
    }
    fn reset(&mut self) {}
}

struct BTFlee;
impl BTNode for BTFlee {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        ctx.desired_move = Some(Dir::away(ctx.self_pos, ctx.target_pos));
        BTStatus::Success
    }
    fn reset(&mut self) {}
}

struct BTAttack;
impl BTNode for BTAttack {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        ctx.wants_attack = true;
        BTStatus::Success
    }
    fn reset(&mut self) {}
}

struct BTPatrol;
impl BTNode for BTPatrol {
    fn tick(&mut self, ctx: &mut AIContext) -> BTStatus {
        if ctx.patrol_points.is_empty() {
            return BTStatus::Failure;
        }
        let tgt = ctx.patrol_points[ctx.patrol_idx];
        if ctx.self_pos == tgt {
            ctx.patrol_idx = (ctx.patrol_idx + 1) % ctx.patrol_points.len();
        }
        ctx.desired_move = Some(Dir::toward(ctx.self_pos, tgt));
        BTStatus::Success
    }
    fn reset(&mut self) {}
}

struct AIContext {
    self_pos: GridPos,
    target_pos: GridPos,
    dist: i32,
    los: bool,
    desired_move: Option<Dir>,
    wants_attack: bool,
    patrol_points: Vec<GridPos>,
    patrol_idx: usize,
}
impl AIContext {
    fn new() -> Self {
        Self {
            self_pos: GridPos::new(0, 0),
            target_pos: GridPos::new(0, 0),
            dist: 999,
            los: false,
            desired_move: None,
            wants_attack: false,
            patrol_points: Vec::new(),
            patrol_idx: 0,
        }
    }
}

fn build_hunter_tree() -> Box<dyn BTNode> {
    // Priority: Attack if adjacent → Chase if visible → Patrol
    Box::new(BTSel::new(vec![
        Box::new(BTSeq::new(vec![
            Box::new(BTCheckClose { threshold: 1 }),
            Box::new(BTAttack),
        ])),
        Box::new(BTSeq::new(vec![Box::new(BTCheckLOS), Box::new(BTMoveToward)])),
        Box::new(BTPatrol),
    ]))
}

fn build_ghost_tree() -> Box<dyn BTNode> {
    // Ghosts always chase, ignoring walls (handled in movement)
    Box::new(BTSel::new(vec![
        Box::new(BTSeq::new(vec![
            Box::new(BTCheckClose { threshold: 1 }),
            Box::new(BTAttack),
        ])),
        Box::new(BTMoveToward),
    ]))
}

// ============================================================================
// CORE TYPES
// ============================================================================
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
struct GridPos {
    x: i32,
    y: i32,
}
impl GridPos {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    fn idx(&self, w: i32) -> usize {
        (self.y * w + self.x) as usize
    }
    fn manhattan(&self, o: GridPos) -> i32 {
        (self.x - o.x).abs() + (self.y - o.y).abs()
    }
    fn euclid(&self, o: GridPos) -> f32 {
        let dx = (self.x - o.x) as f32;
        let dy = (self.y - o.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }
    fn step(&self, d: Dir) -> GridPos {
        let (dx, dy) = d.delta();
        GridPos::new(self.x + dx, self.y + dy)
    }
    fn in_bounds(&self) -> bool {
        self.x >= 0 && self.x < GRID_W && self.y >= 0 && self.y < GRID_H
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}
impl Dir {
    fn delta(&self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }
    fn toward(from: GridPos, to: GridPos) -> Dir {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx.abs() > dy.abs() {
            if dx > 0 {
                Dir::Right
            } else {
                Dir::Left
            }
        } else if dy > 0 {
            Dir::Down
        } else {
            Dir::Up
        }
    }
    fn away(from: GridPos, to: GridPos) -> Dir {
        match Dir::toward(from, to) {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum ItemKind {
    Crystal,
    Key,
    Skull,
    Candle,
    Trap,
    Teleporter,
    Orb,
    Mushroom,
    Stairs,
    LockedDoor,
}
impl ItemKind {
    fn emoji(&self) -> &'static str {
        match self {
            ItemKind::Crystal => "💎",
            ItemKind::Key => "🔑",
            ItemKind::Skull => "💀",
            ItemKind::Candle => "🕯️",
            ItemKind::Trap => "🪤",
            ItemKind::Teleporter => "🌀",
            ItemKind::Orb => "🔮",
            ItemKind::Mushroom => "🍄",
            ItemKind::Stairs => "🪜",
            ItemKind::LockedDoor => "🚪",
        }
    }
    fn color(&self) -> Color {
        match self {
            ItemKind::Crystal => Color::new(0.5, 0.8, 1.0, 1.0),
            ItemKind::Key => Color::new(1.0, 0.9, 0.2, 1.0),
            ItemKind::Skull => Color::new(0.9, 0.9, 0.9, 1.0),
            ItemKind::Candle => Color::new(1.0, 0.8, 0.4, 1.0),
            ItemKind::Trap => Color::new(0.8, 0.4, 0.2, 1.0),
            ItemKind::Teleporter => Color::new(0.6, 0.2, 1.0, 1.0),
            ItemKind::Orb => Color::new(0.8, 0.4, 1.0, 1.0),
            ItemKind::Mushroom => Color::new(0.9, 0.3, 0.5, 1.0),
            ItemKind::Stairs => Color::new(0.9, 0.7, 0.3, 1.0),
            ItemKind::LockedDoor => Color::new(0.7, 0.5, 0.2, 1.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum EnemyKind {
    Eye,    // 👁️ uses D* pathfinding, respects LOS
    Ghost,  // 👻 phases through walls
    Bat,    // 🦇 fast, erratic
}

struct Enemy {
    pos: GridPos,
    kind: EnemyKind,
    tree: Box<dyn BTNode>,
    ctx: AIContext,
    last_known: GridPos,
    move_cd: f32,
    speed: f32,
    hp: f32,
}

struct Item {
    pos: GridPos,
    kind: ItemKind,
    active: bool,
}

// ============================================================================
// D* LITE PATHFINDING
// ============================================================================
#[derive(Clone, Copy)]
struct DNode {
    g: f32,
    rhs: f32,
    ver: u64,
}

#[derive(Eq, PartialEq)]
struct DKey {
    k1: f32,
    k2: f32,
    pos: GridPos,
    ver: u64,
}
impl Ord for DKey {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.k1
            .total_cmp(&o.k1)
            .then_with(|| self.k2.total_cmp(&o.k2))
            .reverse()
    }
}
impl PartialOrd for DKey {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

struct DStarLite {
    nodes: Vec<DNode>,
    km: f32,
    start: GridPos,
    goal: GridPos,
    open: BinaryHeap<DKey>,
}
impl DStarLite {
    fn new(start: GridPos, goal: GridPos, walls: &[bool]) -> Self {
        let size = (GRID_W * GRID_H) as usize;
        let mut s = Self {
            nodes: vec![
                DNode {
                    g: f32::INFINITY,
                    rhs: f32::INFINITY,
                    ver: 0,
                };
                size
            ],
            km: 0.0,
            start,
            goal,
            open: BinaryHeap::new(),
        };
        s.nodes[goal.idx(GRID_W)].rhs = 0.0;
        s.update_vertex(goal, walls);
        s.compute(walls);
        s
    }
    fn key(&self, p: GridPos) -> (f32, f32) {
        let h = p.manhattan(self.start) as f32;
        let i = p.idx(GRID_W);
        let m = self.nodes[i].g.min(self.nodes[i].rhs);
        (m + h + self.km, m)
    }
    fn neighbors(&self, p: GridPos, walls: &[bool]) -> Vec<GridPos> {
        let mut out = Vec::with_capacity(4);
        for d in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
            let n = p.step(d);
            if n.in_bounds() && !walls[n.idx(GRID_W)] {
                out.push(n);
            }
        }
        out
    }
    fn update_vertex(&mut self, u: GridPos, walls: &[bool]) {
        let i = u.idx(GRID_W);
        if u != self.goal {
            let mut m = f32::INFINITY;
            for n in self.neighbors(u, walls) {
                let v = self.nodes[n.idx(GRID_W)].g + 1.0;
                if v < m {
                    m = v;
                }
            }
            self.nodes[i].rhs = m;
        }
        self.nodes[i].ver += 1;
        let k = self.key(u);
        self.open.push(DKey {
            k1: k.0,
            k2: k.1,
            pos: u,
            ver: self.nodes[i].ver,
        });
    }
    fn compute(&mut self, walls: &[bool]) {
        while let Some(top) = self.open.peek() {
            let u = top.pos;
            let i = u.idx(GRID_W);
            if top.ver != self.nodes[i].ver {
                self.open.pop();
                continue;
            }
            let cur = self.key(u);
            if top.k1 < cur.0 || (top.k1 == cur.0 && top.k2 < cur.1) {
                self.open.pop();
                self.open.push(DKey {
                    k1: cur.0,
                    k2: cur.1,
                    pos: u,
                    ver: self.nodes[i].ver,
                });
            } else if self.nodes[i].g > self.nodes[i].rhs {
                self.nodes[i].g = self.nodes[i].rhs;
                self.open.pop();
                for n in self.neighbors(u, walls) {
                    self.update_vertex(n, walls);
                }
            } else {
                self.nodes[i].g = f32::INFINITY;
                self.open.pop();
                let mut ns = self.neighbors(u, walls);
                ns.push(u);
                for n in ns {
                    self.update_vertex(n, walls);
                }
            }
        }
    }
    fn change_goal(&mut self, new_goal: GridPos, walls: &[bool]) {
        let old = self.goal;
        self.nodes[old.idx(GRID_W)].rhs = f32::INFINITY;
        self.update_vertex(old, walls);
        self.goal = new_goal;
        self.nodes[new_goal.idx(GRID_W)].rhs = 0.0;
        self.update_vertex(new_goal, walls);
        for n in self.neighbors(new_goal, walls) {
            self.update_vertex(n, walls);
        }
        self.compute(walls);
    }
    fn change_start(&mut self, new_start: GridPos) {
        self.km += self.start.manhattan(new_start) as f32;
        self.start = new_start;
    }
    fn terrain_change(&mut self, p: GridPos, walls: &[bool]) {
        self.update_vertex(p, walls);
        for n in self.neighbors(p, walls) {
            self.update_vertex(n, walls);
        }
        self.compute(walls);
    }
    fn best_move(&self, from: GridPos, walls: &[bool]) -> Option<GridPos> {
        let mut best = None;
        let mut best_g = f32::INFINITY;
        for n in self.neighbors(from, walls) {
            let g = self.nodes[n.idx(GRID_W)].g;
            if g < best_g {
                best_g = g;
                best = Some(n);
            }
        }
        best
    }
}

// ============================================================================
// BRESENHAM LINE OF SIGHT
// ============================================================================
fn has_los(from: GridPos, to: GridPos, walls: &[bool]) -> bool {
    let mut x0 = from.x;
    let mut y0 = from.y;
    let dx = (to.x - x0).abs();
    let dy = -(to.y - y0).abs();
    let sx = if x0 < to.x { 1 } else { -1 };
    let sy = if y0 < to.y { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 == to.x && y0 == to.y {
            return true;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if x0 == to.x {
                return true;
            }
            err += dy;
            x0 += sx;
        } else if e2 <= dx {
            if y0 == to.y {
                return true;
            }
            err += dx;
            y0 += sy;
        }
        if !(x0 == from.x && y0 == from.y) {
            let p = GridPos::new(x0, y0);
            if p.in_bounds() && walls[p.idx(GRID_W)] {
                return false;
            }
        }
    }
}

// ============================================================================
// CELLULAR AUTOMATA CAVE GENERATION
// ============================================================================
fn generate_cave(floor: i32, rng: &mut DeterministicRng) -> Vec<bool> {
    let mut walls = vec![false; (GRID_W * GRID_H) as usize];
    let threshold = (42 + floor * 2).min(55);
    for i in 0..walls.len() {
        walls[i] = rng.chance(threshold as f32 / 100.0);
    }
    // Border walls
    for y in 0..GRID_H {
        for x in 0..GRID_W {
            if x == 0 || y == 0 || x == GRID_W - 1 || y == GRID_H - 1 {
                walls[GridPos::new(x, y).idx(GRID_W)] = true;
            }
        }
    }
    // Smooth
    for _ in 0..5 {
        let mut next = walls.clone();
        for y in 1..GRID_H - 1 {
            for x in 1..GRID_W - 1 {
                let mut c = 0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        if walls[GridPos::new(x + dx, y + dy).idx(GRID_W)] {
                            c += 1;
                        }
                    }
                }
                let i = GridPos::new(x, y).idx(GRID_W);
                if c >= 5 {
                    next[i] = true;
                } else if c <= 2 {
                    next[i] = false;
                }
            }
        }
        walls = next;
    }
    // Clear spawn areas
    for p in [
        GridPos::new(1, 1),
        GridPos::new(GRID_W - 2, GRID_H - 2),
        GridPos::new(GRID_W - 2, 1),
    ] {
        walls[p.idx(GRID_W)] = false;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let q = GridPos::new(p.x + dx, p.y + dy);
                if q.in_bounds() {
                    walls[q.idx(GRID_W)] = false;
                }
            }
        }
    }
    walls
}

// ============================================================================
// PROCEDURAL AUDIO (8-bit synthesis)
// ============================================================================
struct AudioService {
    master: f32,
    sfx: f32,
    muted: bool,
    footstep: Option<macroquad::audio::Sound>,
    pickup: Option<macroquad::audio::Sound>,
    damage: Option<macroquad::audio::Sound>,
    wall: Option<macroquad::audio::Sound>,
    teleport: Option<macroquad::audio::Sound>,
    whisper: Option<macroquad::audio::Sound>,
    heartbeat: Option<macroquad::audio::Sound>,
    ambient: Option<macroquad::audio::Sound>,
}
impl AudioService {
    fn new() -> Self {
        Self {
            master: 1.0,
            sfx: 0.7,
            muted: false,
            footstep: None,
            pickup: None,
            damage: None,
            wall: None,
            teleport: None,
            whisper: None,
            heartbeat: None,
            ambient: None,
        }
    }
    async fn load_samples(&mut self) {
        self.footstep = Some(Self::synth(0.1, |t, _| {
            let env = (1.0 - t / 0.1).powi(2);
            let n = ((t * 1103515245.0) as i32 % 1000) as f32 / 500.0 - 1.0;
            n * env * 0.3
        }));
        self.pickup = Some(Self::synth(0.3, |t, _| {
            let notes = [440.0, 554.0, 659.0, 880.0];
            let idx = ((t / 0.3) * 4.0) as usize;
            let f = notes[idx.min(3)];
            let sq = if (t * f).sin() > 0.0 { 1.0 } else { -1.0 };
            sq * (1.0 - t / 0.3).sqrt() * 0.4
        }));
        self.damage = Some(Self::synth(0.4, |t, _| {
            let f = 200.0 - t * 300.0;
            let saw = ((t * f) % 1.0) * 2.0 - 1.0;
            let d = (saw * 2.0).tanh();
            d * (1.0 - t / 0.4).powi(2) * 0.5
        }));
        self.wall = Some(Self::synth(0.3, |t, _| {
            let imp = ((t * 100.0).sin() * (1.0 - t / 0.3).powi(3)).signum() * 0.5;
            let n = ((t * 1103515245.0) as i32 % 1000) as f32 / 500.0 - 1.0;
            let ne = (1.0 - t / 0.3).powi(4) * 0.3;
            (imp + n * ne) * 0.5
        }));
        self.teleport = Some(Self::synth(0.5, |t, _| {
            let f = 100.0 * (t / 0.5 * 5.0).exp();
            let sq = if (t * f).sin() > 0.0 { 1.0 } else { -1.0 };
            sq * (1.0 - t / 0.5).powf(0.7) * 0.4
        }));
        self.whisper = Some(Self::synth(0.8, |t, _| {
            let n = ((t * 1103515245.0) as i32 % 1000) as f32 / 500.0 - 1.0;
            let f = (t * 1000.0 * std::f32::consts::TAU).sin().abs();
            let env = (1.0 - t / 0.8).sqrt() * (t * 8.0).sin().abs();
            n * f * env * 0.3
        }));
        self.heartbeat = Some(Self::synth(0.6, |t, _| {
            let p1 = if t < 0.1 {
                (1.0 - t / 0.1).powi(2)
            } else {
                0.0
            };
            let p2 = if t > 0.2 && t < 0.3 {
                (1.0 - (t - 0.2) / 0.1).powi(2)
            } else {
                0.0
            };
            let th = ((t * 60.0).sin() * (p1 + p2)).signum() * (p1 + p2);
            th * 0.5
        }));
        self.ambient = Some(Self::synth(4.0, |t, _| {
            let bf = 55.0 + (t * 0.1).sin() * 5.0;
            let sq = if (t * bf).sin() > 0.0 { 1.0 } else { -1.0 };
            let h1 = ((t * bf * 1.5).sin() * 0.3).signum() * 0.3;
            let h2 = ((t * bf * 2.1).sin() * 0.2).signum() * 0.2;
            let env = 0.3 + (t * 0.5).sin() * 0.15;
            (sq + h1 + h2) * env * 0.2
        }));
    }
    fn synth<F: Fn(f32, f32) -> f32>(dur: f32, f: F) -> macroquad::audio::Sound {
        let sr = 22050u32;
        let n = (sr as f32 * dur) as usize;
        let mut buf = vec![0i16; n * 2];
        for i in 0..n {
            let t = i as f32 / sr as f32;
            let s = f(t, dur);
            let v = (s * 16000.0).clamp(-32000.0, 32000.0) as i16;
            buf[i * 2] = v;
            buf[i * 2 + 1] = v;
        }
        macroquad::audio::Sound::from_wave(sr, &buf)
    }
    fn vol(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.master * self.sfx
        }
    }
    fn play(&self, s: &Option<macroquad::audio::Sound>) {
        if let Some(sound) = s {
            macroquad::audio::play_sound(
                sound.clone(),
                macroquad::audio::PlaySoundParams {
                    looped: false,
                    volume: self.vol(),
                },
            );
        }
    }
    fn play_ambient(&self) {
        if let Some(s) = &self.ambient {
            macroquad::audio::play_sound(
                s.clone(),
                macroquad::audio::PlaySoundParams {
                    looped: true,
                    volume: self.vol() * 0.5,
                },
            );
        }
    }
}

// ============================================================================
// PERSISTENCE (Save Data)
// ============================================================================
#[derive(Serialize, Deserialize, Clone, Default)]
struct SaveData {
    high_scores: Vec<HighScore>,
    achievements: Vec<String>,
    skill_points: i32,
    skills: Vec<String>,
    stats: PersistentStats,
}
#[derive(Serialize, Deserialize, Clone)]
struct HighScore {
    floor: i32,
    steps: i32,
    score: i32,
}
#[derive(Serialize, Deserialize, Clone, Default)]
struct PersistentStats {
    runs: i32,
    floors: i32,
    items: i32,
    time: f32,
}
impl SaveData {
    fn load() -> Self {
        std::fs::read_to_string("save.json")
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self) {
        if let Ok(j) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write("save.json", j);
        }
    }
    fn add_score(&mut self, floor: i32, steps: i32, score: i32) {
        self.high_scores.push(HighScore {
            floor,
            steps,
            score,
        });
        self.high_scores.sort_by(|a, b| b.score.cmp(&a.score));
        self.high_scores.truncate(10);
        self.save();
    }
    fn unlock(&mut self, name: &str) -> bool {
        if !self.achievements.contains(&name.to_string()) {
            self.achievements.push(name.to_string());
            self.skill_points += 1;
            self.save();
            true
        } else {
            false
        }
    }
    fn learn_skill(&mut self, s: &str) -> bool {
        if self.skill_points > 0 && !self.skills.contains(&s.to_string()) {
            self.skills.push(s.to_string());
            self.skill_points -= 1;
            self.save();
            true
        } else {
            false
        }
    }
    fn has_skill(&self, s: &str) -> bool {
        self.skills.contains(&s.to_string())
    }
}

// ============================================================================
// ACHIEVEMENTS
// ============================================================================
struct AchievementDef {
    id: &'static str,
    name: &'static str,
    emoji: &'static str,
}
const ACHIEVEMENTS: &[AchievementDef] = &[
    AchievementDef {
        id: "first_steps",
        name: "First Steps",
        emoji: "🏅",
    },
    AchievementDef {
        id: "key_master",
        name: "Key Master",
        emoji: "🗝️",
    },
    AchievementDef {
        id: "wall_builder",
        name: "Wall Builder",
        emoji: "🧱",
    },
    AchievementDef {
        id: "shadow_walker",
        name: "Shadow Walker",
        emoji: "👁️",
    },
    AchievementDef {
        id: "crystal_hunter",
        name: "Crystal Hunter",
        emoji: "💎",
    },
    AchievementDef {
        id: "deep_delver",
        name: "Deep Delver",
        emoji: "🪜",
    },
    AchievementDef {
        id: "combo_king",
        name: "Combo King",
        emoji: "🔥",
    },
    AchievementDef {
        id: "survivor",
        name: "Survivor",
        emoji: "💪",
    },
];

// ============================================================================
// WEATHER
// ============================================================================
#[derive(Clone, Copy, PartialEq)]
enum Weather {
    Clear,
    Rain,
    Fog,
    Storm,
}
struct WeatherSystem {
    current: Weather,
    intensity: f32,
    timer: f32,
    drops: Vec<(f32, f32, f32, f32)>,
}
impl WeatherSystem {
    fn new() -> Self {
        Self {
            current: Weather::Clear,
            intensity: 0.0,
            timer: 15.0,
            drops: Vec::new(),
        }
    }
    fn update(&mut self, dt: f32, rng: &mut DeterministicRng) {
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.current = match rng.range(0, 3) {
                0 => Weather::Clear,
                1 => Weather::Rain,
                2 => Weather::Fog,
                _ => Weather::Storm,
            };
            self.intensity = 0.5 + rng.next_f32() * 0.5;
            self.timer = 15.0 + rng.next_f32() * 20.0;
        }
        match self.current {
            Weather::Rain | Weather::Storm => {
                let count = (self.intensity * 15.0) as i32;
                for _ in 0..count {
                    let x = rng.next_f32() * screen_width();
                    let vx = if self.current == Weather::Storm {
                        -80.0
                    } else {
                        -30.0
                    };
                    let vy = 350.0 + rng.next_f32() * 150.0;
                    self.drops.push((x, -10.0, vx, vy));
                }
                self.drops.retain_mut(|d| {
                    d.0 += d.2 * dt;
                    d.1 += d.3 * dt;
                    d.1 < screen_height()
                });
                if self.drops.len() > 800 {
                    self.drops.drain(0..200);
                }
            }
            _ => {
                self.drops.clear();
            }
        }
    }
    fn visibility(&self) -> f32 {
        match self.current {
            Weather::Clear => 1.0,
            Weather::Rain => 0.85,
            Weather::Fog => 0.6,
            Weather::Storm => 0.7,
        }
    }
    fn render(&self) {
        match self.current {
            Weather::Rain | Weather::Storm => {
                for (x, y, _, _) in &self.drops {
                    draw_line(*x, *y, *x - 2.0, *y + 10.0, 1.0, Color::new(0.6, 0.7, 0.9, 0.4));
                }
            }
            Weather::Fog => {
                draw_rectangle(
                    0.0,
                    0.0,
                    screen_width(),
                    screen_height(),
                    Color::new(0.5, 0.5, 0.6, 0.15),
                );
            }
            _ => {}
        }
    }
}

// ============================================================================
// COMBO SYSTEM
// ============================================================================
struct ComboSystem {
    count: i32,
    timer: f32,
    max: i32,
}
impl ComboSystem {
    fn new() -> Self {
        Self {
            count: 0,
            timer: 0.0,
            max: 0,
        }
    }
    fn hit(&mut self) -> i32 {
        self.count += 1;
        self.timer = 2.0;
        if self.count > self.max {
            self.max = self.count;
        }
        self.count
    }
    fn update(&mut self, dt: f32) {
        if self.timer > 0.0 {
            self.timer -= dt;
            if self.timer <= 0.0 {
                self.count = 0;
            }
        }
    }
    fn mult(&self) -> f32 {
        1.0 + self.count as f32 * 0.1
    }
}

// ============================================================================
// WORLD (ECS-lite)
// ============================================================================
struct World {
    walls: Vec<bool>,
    player: GridPos,
    prev_player: GridPos,
    sanity: f32,
    max_sanity: f32,
    keys: i32,
    crystals: i32,
    skulls_hit: i32,
    teleports_used: i32,
    walls_placed: i32,
    steps: i32,
    floor: i32,
    items: Vec<Item>,
    enemies: Vec<Enemy>,
    dstar: DStarLite,
    exit: GridPos,
    day_night: f32,
    invincible_until: f32,
    speed_until: f32,
    last_known_player: GridPos,
    has_los: bool,
    wall_cd: f32,
    move_cd: f32,
    heartbeat_cd: f32,
    blood: HashSet<GridPos>,
    combo: ComboSystem,
    weather: WeatherSystem,
    particles: ParticlePool,
    spatial: SpatialHash,
    commands: CommandQueue,
    pending_achievements: Vec<String>,
    screen_shake: f32,
    transition: f32,
    transitioning: bool,
}

impl World {
    fn new(seed: u64, save: &SaveData) -> Self {
        let mut rng = DeterministicRng::new(seed);
        let walls = generate_cave(1, &mut rng);
        let mut items = Vec::new();
        let mut enemies = Vec::new();
        Self::populate_floor(&walls, &mut items, &mut enemies, 1, &mut rng);

        let exit = items
            .iter()
            .find(|i| i.kind == ItemKind::Stairs && i.active)
            .map(|i| i.pos)
            .unwrap_or(GridPos::new(GRID_W - 2, GRID_H - 2));

        let player = GridPos::new(1, 1);
        let enemy_start = GridPos::new(GRID_W - 2, GRID_H - 2);

        let mut eye_ctx = AIContext::new();
        eye_ctx.self_pos = enemy_start;
        eye_ctx.target_pos = player;
        eye_ctx.patrol_points = vec![
            GridPos::new(2, 2),
            GridPos::new(GRID_W - 3, 2),
            GridPos::new(GRID_W - 3, GRID_H - 3),
            GridPos::new(2, GRID_H - 3),
        ];

        let mut ghost_ctx = AIContext::new();
        ghost_ctx.self_pos = GridPos::new(GRID_W - 2, 1);
        ghost_ctx.target_pos = player;

        enemies.push(Enemy {
            pos: enemy_start,
            kind: EnemyKind::Eye,
            tree: build_hunter_tree(),
            ctx: eye_ctx,
            last_known: player,
            move_cd: 0.0,
            speed: 0.25,
            hp: 3.0,
        });
        enemies.push(Enemy {
            pos: GridPos::new(GRID_W - 2, 1),
            kind: EnemyKind::Ghost,
            tree: build_ghost_tree(),
            ctx: ghost_ctx,
            last_known: player,
            move_cd: 0.0,
            speed: 0.35,
            hp: 2.0,
        });

        let dstar = DStarLite::new(enemy_start, player, &walls);

        let base_sanity = if save.has_skill("tough_mind") {
            120.0
        } else {
            100.0
        };

        Self {
            walls,
            player,
            prev_player: player,
            sanity: base_sanity,
            max_sanity: base_sanity,
            keys: 0,
            crystals: 0,
            skulls_hit: 0,
            teleports_used: 0,
            walls_placed: 0,
            steps: 0,
            floor: 1,
            items,
            enemies,
            dstar,
            exit,
            day_night: 0.0,
            invincible_until: 0.0,
            speed_until: 0.0,
            last_known_player: player,
            has_los: true,
            wall_cd: 0.0,
            move_cd: 0.0,
            heartbeat_cd: 0.0,
            blood: HashSet::new(),
            combo: ComboSystem::new(),
            weather: WeatherSystem::new(),
            particles: ParticlePool::new(PARTICLE_POOL_SIZE),
            spatial: SpatialHash::new(2),
            commands: CommandQueue::new(),
            pending_achievements: Vec::new(),
            screen_shake: 0.0,
            transition: 0.0,
            transitioning: false,
        }
    }

    fn populate_floor(
        walls: &[bool],
        items: &mut Vec<Item>,
        _enemies: &mut Vec<Enemy>,
        floor: i32,
        rng: &mut DeterministicRng,
    ) {
        items.clear();
        let count = 10 + floor * 2;
        for _ in 0..count {
            for _ in 0..50 {
                let x = rng.range(1, GRID_W - 2);
                let y = rng.range(1, GRID_H - 2);
                let p = GridPos::new(x, y);
                if !walls[p.idx(GRID_W)]
                    && !(x <= 2 && y <= 2)
                    && !(x >= GRID_W - 3 && y >= GRID_H - 3)
                {
                    let kind = match rng.range(0, 9) {
                        0 => ItemKind::Crystal,
                        1 => ItemKind::Key,
                        2 => ItemKind::Skull,
                        3 => ItemKind::Candle,
                        4 => ItemKind::Trap,
                        5 => ItemKind::Teleporter,
                        6 => ItemKind::Orb,
                        7 => ItemKind::Mushroom,
                        _ => ItemKind::Crystal,
                    };
                    items.push(Item {
                        pos: p,
                        kind,
                        active: true,
                    });
                    break;
                }
            }
        }
        // Stairs
        for _ in 0..100 {
            let x = rng.range(GRID_W / 2, GRID_W - 2);
            let y = rng.range(GRID_H / 2, GRID_H - 2);
            let p = GridPos::new(x, y);
            if !walls[p.idx(GRID_W)] {
                items.push(Item {
                    pos: p,
                    kind: ItemKind::Stairs,
                    active: true,
                });
                break;
            }
        }
        // Locked door
        if rng.chance(0.5) {
            for _ in 0..50 {
                let x = rng.range(3, GRID_W - 4);
                let y = rng.range(3, GRID_H - 4);
                let p = GridPos::new(x, y);
                if !walls[p.idx(GRID_W)] {
                    items.push(Item {
                        pos: p,
                        kind: ItemKind::LockedDoor,
                        active: true,
                    });
                    break;
                }
            }
        }
    }

    fn try_move_player(&mut self, d: Dir) {
        let next = self.player.step(d);
        if !next.in_bounds() {
            return;
        }
        let idx = next.idx(GRID_W);
        if self.walls[idx] {
            return;
        }
        // Check locked door
        for it in &self.items {
            if it.active && it.kind == ItemKind::LockedDoor && it.pos == next {
                if self.keys > 0 {
                    self.keys -= 1;
                    // Mark door inactive via mutable iteration below
                } else {
                    return;
                }
            }
        }
        // Remove door if opened
        for it in &mut self.items {
            if it.active && it.kind == ItemKind::LockedDoor && it.pos == next {
                it.active = false;
            }
        }
        self.prev_player = self.player;
        self.player = next;
        self.steps += 1;
        self.sanity -= 1.5;
        self.on_player_moved();
    }

    fn place_wall(&mut self, p: GridPos) {
        if !p.in_bounds() {
            return;
        }
        if self.walls[p.idx(GRID_W)] {
            return;
        }
        if p == self.player {
            return;
        }
        self.walls[p.idx(GRID_W)] = true;
        self.walls_placed += 1;
        self.dstar.terrain_change(p, &self.walls);
        let px = OFFSET_X + p.x as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        let py = OFFSET_Y + p.y as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        self.particles.spawn(
            Vec2::new(px, py),
            Vec2::new(0.0, -40.0),
            "🧱",
            WHITE,
            0.8,
            28.0,
            0.0,
        );
    }

    fn use_item(&mut self, _kind: ItemKind) {
        // Reserved for consumables
    }

    fn on_player_moved(&mut self) {
        self.dstar.change_goal(self.player, &self.walls);
        // Collect items
        for it in &mut self.items {
            if it.active && it.pos == self.player {
                self.collect_item(it.kind);
                it.active = false;
            }
        }
    }

    fn collect_item(&mut self, kind: ItemKind) {
        let px = OFFSET_X + self.player.x as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        let py = OFFSET_Y + self.player.y as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        match kind {
            ItemKind::Crystal => {
                self.crystals += 1;
                self.sanity = (self.sanity + 20.0).min(self.max_sanity);
                self.particles.spawn(
                    Vec2::new(px, py),
                    Vec2::new(0.0, -60.0),
                    "✨",
                    Color::new(0.5, 0.8, 1.0, 1.0),
                    1.0,
                    24.0,
                    0.0,
                );
                self.combo.hit();
            }
            ItemKind::Key => {
                self.keys += 1;
                self.particles.spawn(
                    Vec2::new(px, py),
                    Vec2::new(0.0, -60.0),
                    "🔑",
                    GOLD,
                    1.0,
                    24.0,
                    0.0,
                );
                self.combo.hit();
            }
            ItemKind::Skull => {
                self.take_damage(25.0, "💀".into());
                self.skulls_hit += 1;
            }
            ItemKind::Trap => {
                self.take_damage(15.0, "🪤".into());
                self.blood.insert(self.player);
            }
            ItemKind::Teleporter => {
                self.teleports_used += 1;
                // Random open tile
                for _ in 0..200 {
                    let x = rand_range(1, GRID_W - 2);
                    let y = rand_range(1, GRID_H - 2);
                    let p = GridPos::new(x, y);
                    if !self.walls[p.idx(GRID_W)] {
                        self.player = p;
                        self.prev_player = p;
                        break;
                    }
                }
                self.particles.spawn(
                    Vec2::new(px, py),
                    Vec2::new(0.0, 0.0),
                    "🌀",
                    Color::new(0.6, 0.2, 1.0, 1.0),
                    1.2,
                    32.0,
                    0.0,
                );
            }
            ItemKind::Orb => {
                self.invincible_until = self.day_night + 5.0;
                self.particles.spawn(
                    Vec2::new(px, py),
                    Vec2::new(0.0, -40.0),
                    "🔮",
                    Color::new(0.8, 0.4, 1.0, 1.0),
                    1.5,
                    28.0,
                    0.0,
                );
            }
            ItemKind::Mushroom => {
                self.sanity = (self.sanity + 10.0).min(self.max_sanity);
                self.speed_until = self.day_night + 4.0;
                self.particles.spawn(
                    Vec2::new(px, py),
                    Vec2::new(0.0, -40.0),
                    "🍄",
                    Color::new(0.9, 0.3, 0.5, 1.0),
                    1.0,
                    26.0,
                    0.0,
                );
            }
            ItemKind::Stairs => {
                self.descend();
            }
            ItemKind::Candle | ItemKind::LockedDoor => {}
        }
    }

    fn descend(&mut self) {
        self.floor += 1;
        self.transitioning = true;
        self.transition = 0.0;
        let mut rng = DeterministicRng::new(self.floor as u64 * 9173 + 424242);
        self.walls = generate_cave(self.floor, &mut rng);
        self.items.clear();
        self.enemies.clear();
        Self::populate_floor(&self.walls, &mut self.items, &mut self.enemies, self.floor, &mut rng);
        self.exit = self
            .items
            .iter()
            .find(|i| i.kind == ItemKind::Stairs && i.active)
            .map(|i| i.pos)
            .unwrap_or(GridPos::new(GRID_W - 2, GRID_H - 2));
        self.player = GridPos::new(1, 1);
        self.prev_player = self.player;
        self.sanity = (self.sanity + 30.0).min(self.max_sanity);

        let enemy_start = GridPos::new(GRID_W - 2, GRID_H - 2);
        let mut ctx = AIContext::new();
        ctx.self_pos = enemy_start;
        ctx.target_pos = self.player;
        ctx.patrol_points = vec![
            GridPos::new(2, 2),
            GridPos::new(GRID_W - 3, 2),
            GridPos::new(GRID_W - 3, GRID_H - 3),
            GridPos::new(2, GRID_H - 3),
        ];
        self.enemies.push(Enemy {
            pos: enemy_start,
            kind: EnemyKind::Eye,
            tree: build_hunter_tree(),
            ctx,
            last_known: self.player,
            move_cd: 0.0,
            speed: 0.25 + self.floor as f32 * 0.03,
            hp: 3.0 + self.floor as f32,
        });
        if self.floor >= 2 {
            let mut gctx = AIContext::new();
            gctx.self_pos = GridPos::new(GRID_W - 2, 1);
            gctx.target_pos = self.player;
            self.enemies.push(Enemy {
                pos: GridPos::new(GRID_W - 2, 1),
                kind: EnemyKind::Ghost,
                tree: build_ghost_tree(),
                ctx: gctx,
                last_known: self.player,
                move_cd: 0.0,
                speed: 0.35 + self.floor as f32 * 0.02,
                hp: 2.0,
            });
        }
        self.dstar = DStarLite::new(enemy_start, self.player, &self.walls);
        self.last_known_player = self.player;
        self.blood.clear();
    }

    fn take_damage(&mut self, amount: f32, source: String) {
        if self.day_night < self.invincible_until {
            return;
        }
        self.sanity -= amount;
        self.screen_shake = 0.4;
        let px = OFFSET_X + self.player.x as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        let py = OFFSET_Y + self.player.y as f32 * TILE_SIZE + TILE_SIZE / 2.0;
        self.particles.spawn(
            Vec2::new(px, py),
            Vec2::new(0.0, -30.0),
            "💥",
            RED,
            0.8,
            32.0,
            0.0,
        );
    }

    fn update_enemies(&mut self, dt: f32, audio: &AudioService) {
        for enemy in &mut self.enemies {
            enemy.move_cd -= dt;
            if enemy.move_cd > 0.0 {
                continue;
            }
            let los = has_los(enemy.pos, self.player, &self.walls);
            let dist = enemy.pos.manhattan(self.player);
            enemy.ctx.self_pos = enemy.pos;
            enemy.ctx.target_pos = if los {
                self.player
            } else {
                enemy.last_known
            };
            enemy.ctx.dist = dist;
            enemy.ctx.los = los;
            enemy.ctx.desired_move = None;
            enemy.ctx.wants_attack = false;

            let _ = enemy.tree.tick(&mut enemy.ctx);

            if let Some(d) = enemy.ctx.desired_move {
                let next = enemy.pos.step(d);
                let can_move = match enemy.kind {
                    EnemyKind::Eye | EnemyKind::Bat => {
                        next.in_bounds() && !self.walls[next.idx(GRID_W)]
                    }
                    EnemyKind::Ghost => next.in_bounds(),
                };
                if can_move {
                    enemy.pos = next;
                    enemy.move_cd = enemy.speed;
                    if enemy.kind == EnemyKind::Eye {
                        self.dstar.change_start(enemy.pos);
                    }
                }
            }

            if enemy.ctx.wants_attack && dist <= 1 {
                if self.player.manhattan(enemy.pos) <= 1 {
                    self.take_damage(20.0, format!("{:?}", enemy.kind));
                    audio.play(&audio.damage);
                }
            }

            if los {
                enemy.last_known = self.player;
            }
        }
    }

    fn update(&mut self, dt: f32, services: &Services, save: &SaveData) {
        // Cooldowns
        if self.move_cd > 0.0 {
            self.move_cd -= dt;
        }
        if self.wall_cd > 0.0 {
            self.wall_cd -= dt;
        }
        if self.heartbeat_cd > 0.0 {
            self.heartbeat_cd -= dt;
        }
        if self.screen_shake > 0.0 {
            self.screen_shake -= dt;
        }
        if self.transitioning {
            self.transition += dt * 2.0;
            if self.transition >= 1.0 {
                self.transitioning = false;
                self.transition = 0.0;
            }
        }

        // Day/night
        self.day_night = (self.day_night + dt * 0.03) % 1.0;
        let night_factor = (self.day_night * std::f32::consts::PI).sin().abs();

        // Weather
        self.weather
            .update(dt, &mut services.rng.borrow_mut());

        // Combo
        self.combo.update(dt);

        // Passive sanity drain
        self.sanity -= dt * (1.5 + night_factor * 1.0) * self.weather.visibility();

        // Heartbeat
        let eye = self
            .enemies
            .iter()
            .find(|e| e.kind == EnemyKind::Eye);
        if let Some(e) = eye {
            let d = e.pos.euclid(self.player);
            if d < 6.0 && self.heartbeat_cd <= 0.0 {
                let interval = 0.3 + (d / 6.0) * 0.5;
                self.heartbeat_cd = interval;
                services.audio.borrow().play(&services.audio.borrow().heartbeat);
            }
        }

        // LOS
        if let Some(e) = self.enemies.iter().find(|e| e.kind == EnemyKind::Eye) {
            self.has_los = has_los(e.pos, self.player, &self.walls);
            if self.has_los {
                self.last_known_player = self.player;
            }
            let goal = if self.has_los {
                self.player
            } else {
                self.last_known_player
            };
            if goal != self.dstar.goal {
                self.dstar.change_goal(goal, &self.walls);
            }
        }

        // Update enemies
        self.update_enemies(dt, &services.audio.borrow());

        // Particles
        self.particles.update(dt);

        // Death
        if self.sanity <= 0.0 {
            self.sanity = 0.0;
        }

        // Achievements
        self.check_achievements(save);
    }

    fn check_achievements(&mut self, save: &SaveData) {
        let mut svc_events = std::cell::RefCell::new(Vec::new());
        if self.steps >= 10 {
            svc_events.borrow_mut().push("first_steps");
        }
        if self.keys >= 5 {
            svc_events.borrow_mut().push("key_master");
        }
        if self.walls_placed >= 10 {
            svc_events.borrow_mut().push("wall_builder");
        }
        if self.sanity < 20.0 && self.sanity > 0.0 {
            svc_events.borrow_mut().push("shadow_walker");
        }
        if self.crystals >= 10 {
            svc_events.borrow_mut().push("crystal_hunter");
        }
        if self.floor >= 5 {
            svc_events.borrow_mut().push("deep_delver");
        }
        if self.combo.max >= 5 {
            svc_events.borrow_mut().push("combo_king");
        }
        if self.steps >= 200 {
            svc_events.borrow_mut().push("survivor");
        }
        for id in svc_events.borrow().iter() {
            if !save.achievements.contains(&id.to_string()) {
                self.pending_achievements.push(id.to_string());
            }
        }
    }

    fn score(&self) -> i32 {
        (self.floor * 1000
            + self.steps * 10
            + self.crystals * 50
            + self.keys * 100
            + self.walls_placed * 20) as i32
    }
}

fn rand_range(min: i32, max: i32) -> i32 {
    // Simple deterministic-free rand for teleport; acceptable
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    min + (t as i32 % (max - min + 1)).abs()
}

// ============================================================================
// PATTERN #3: HIERARCHICAL STATE MACHINE
// ============================================================================
#[derive(Clone, Copy, PartialEq)]
enum Scene {
    MainMenu,
    Playing,
    Paused,
    SkillTree,
    HighScores,
    GameOver,
    Victory,
}

// ============================================================================
// RENDERING
// ============================================================================
fn draw_grid(world: &World, font: &Font) {
    let vis_base = 5.0 * world.weather.visibility();
    let night = (world.day_night * std::f32::consts::PI).sin().abs();
    let shake_x = if world.screen_shake > 0.0 {
        (world.day_night * 1000.0).sin() as f32 * world.screen_shake * 10.0
    } else {
        0.0
    };
    let shake_y = if world.screen_shake > 0.0 {
        (world.day_night * 1200.0).cos() as f32 * world.screen_shake * 10.0
    } else {
        0.0
    };

    for y in 0..GRID_H {
        for x in 0..GRID_W {
            let p = GridPos::new(x, y);
            let d = p.euclid(world.player);
            if d > vis_base {
                continue;
            }
            let alpha = (1.0 - d / vis_base).clamp(0.0, 1.0) * (1.0 - night * 0.3);
            let mut emoji = "🌫️";
            let mut color = Color::new(1.0, 1.0, 1.0, alpha);

            let idx = p.idx(GRID_W);
            if world.walls[idx] {
                emoji = "🧱";
                color = Color::new(0.35, 0.25, 0.25, alpha);
            } else if p == world.player {
                emoji = if world.sanity < 30.0 { "🩸" } else { "🚶" };
                if world.day_night < world.invincible_until {
                    color = Color::new(0.8, 0.4, 1.0, alpha);
                }
            } else if world.blood.contains(&p) {
                emoji = "🩸";
                color = Color::new(0.6, 0.0, 0.0, alpha * 0.6);
            } else {
                // Items
                let mut found = false;
                for it in &world.items {
                    if it.active && it.pos == p {
                        emoji = it.kind.emoji();
                        let c = it.kind.color();
                        color = Color::new(c.r, c.g, c.b, alpha);
                        found = true;
                        break;
                    }
                }
                if !found {
                    // Enemies
                    for e in &world.enemies {
                        if e.pos == p {
                            emoji = match e.kind {
                                EnemyKind::Eye => {
                                    if world.sanity < 40.0 {
                                        "👹"
                                    } else {
                                        "👁️"
                                    }
                                }
                                EnemyKind::Ghost => "👻",
                                EnemyKind::Bat => "🦇",
                            };
                            color = Color::new(1.0, 0.1, 0.1, alpha);
                            found = true;
                            break;
                        }
                    }
                }
                if !found {
                    // Ambient decor
                    if (x + y) % 11 == 0 {
                        emoji = "🌲";
                        color = Color::new(0.2, 0.4, 0.2, alpha);
                    } else if (x * y) % 13 == 0 {
                        emoji = "🪦";
                        color = Color::new(0.5, 0.5, 0.5, alpha);
                    } else if (x + y * 3) % 17 == 0 {
                        emoji = "🕸️";
                        color = Color::new(0.6, 0.6, 0.6, alpha * 0.7);
                    }
                }
            }

            // Hallucinations
            if world.sanity < 50.0 && d <= 3.0 {
                let n = ((x as f64 * 12.345 + y as f64 * 67.89) + get_time() * 4.0).sin();
                if n > (0.8 - (100.0 - world.sanity) / 100.0) {
                    emoji = "👁️";
                    color = Color::new(1.0, 0.0, 0.0, alpha * 0.6);
                }
            }

            draw_text_ex(
                emoji,
                OFFSET_X + x as f32 * TILE_SIZE + shake_x,
                OFFSET_Y + y as f32 * TILE_SIZE + 26.0 + shake_y,
                TextParams {
                    font: Some(font.clone()),
                    font_size: 26,
                    color,
                    ..Default::default()
                },
            );
        }
    }
}

fn draw_hud(world: &World, font: &Font, save: &SaveData) {
    root_ui().window(
        hash!(),
        Vec2::new(10.0, 10.0),
        Vec2::new(230.0, 240.0),
        |ui| {
            ui.label(None, &format!("FLOOR {} | STEP {}", world.floor, world.steps));
            ui.label(None, &format!("🔑 {}  💎 {}  🧱 {}", world.keys, world.crystals, world.walls_placed));
            let los = if world.has_los { "🟢 SEEN" } else { "🔴 HIDDEN" };
            ui.label(None, &format!("LOS: {}", los));
            let tod = if (world.day_night * std::f32::consts::PI).sin().abs() > 0.5 {
                "🌙 NIGHT"
            } else {
                "☀️ DAY"
            };
            let wth = match world.weather.current {
                Weather::Clear => "☀️",
                Weather::Rain => "🌧️",
                Weather::Fog => "🌫️",
                Weather::Storm => "⛈️",
            };
            ui.label(None, &format!("{} {}", tod, wth));
            ui.label(None, &format!("COMBO x{} ({:.1}x)", world.combo.count, world.combo.mult()));
            ui.label(None, "SANITY:");
            let bw = 200.0;
            let fw = (world.sanity / world.max_sanity).clamp(0.0, 1.0) * bw;
            ui.draw_rect(Rect::new(0.0, ui.cursor().y, bw, 14.0), None, DARKGRAY);
            let c = if world.sanity < 30.0 {
                RED
            } else if world.sanity < 60.0 {
                YELLOW
            } else {
                GREEN
            };
            ui.draw_rect(Rect::new(0.0, ui.cursor().y - 14.0, fw, 14.0), None, c);
            ui.cursor_mut().y += 18.0;
            ui.label(
                None,
                &format!("SCORE: {}", world.score()),
            );
            ui.label(
                None,
                &format!("SKILL PTS: {}", save.skill_points),
            );
        },
    );
}

fn draw_minimap(world: &World) {
    let size = 130.0;
    let ts = size / GRID_W as f32;
    let x = screen_width() - size - 20.0;
    let y = 20.0;
    draw_rectangle(
        x - 5.0,
        y - 5.0,
        size + 10.0,
        size + 10.0,
        Color::new(0.0, 0.0, 0.0, 0.85),
    );
    draw_rectangle_lines(
        x - 5.0,
        y - 5.0,
        size + 10.0,
        size + 10.0,
        2.0,
        WHITE,
    );
    for wy in 0..GRID_H {
        for wx in 0..GRID_W {
            let p = GridPos::new(wx, wy);
            let d = p.manhattan(world.player);
            if d > 8 {
                continue;
            }
            if world.walls[p.idx(GRID_W)] {
                draw_rectangle(
                    x + wx as f32 * ts,
                    y + wy as f32 * ts,
                    ts,
                    ts,
                    Color::new(0.4, 0.3, 0.3, 0.9),
                );
            }
        }
    }
    // Exit
    draw_rectangle(
        x + world.exit.x as f32 * ts,
        y + world.exit.y as f32 * ts,
        ts,
        ts,
        GOLD,
    );
    // Enemies in LOS
    for e in &world.enemies {
        if has_los(e.pos, world.player, &world.walls) {
            draw_rectangle(
                x + e.pos.x as f32 * ts,
                y + e.pos.y as f32 * ts,
                ts,
                ts,
                RED,
            );
        }
    }
    // Player
    draw_rectangle(
        x + world.player.x as f32 * ts,
        y + world.player.y as f32 * ts,
        ts,
        ts,
        GREEN,
    );
    draw_text_ex(
        "MAP",
        x,
        y - 8.0,
        TextParams {
            font_size: 14,
            color: WHITE,
            ..Default::default()
        },
    );
}

fn draw_crt() {
    for y in (0..screen_height() as i32).step_by(4) {
        draw_rectangle(
            0.0,
            y as f32,
            screen_width(),
            1.5,
            Color::new(0.0, 0.0, 0.0, 0.12),
        );
    }
    for i in 0..15 {
        let a = (i as f32 / 15.0) * 0.25;
        let s = (i as f32 / 15.0) * 60.0;
        draw_rectangle_lines(
            s,
            s,
            screen_width() - s * 2.0,
            screen_height() - s * 2.0,
            2.0,
            Color::new(0.0, 0.0, 0.0, a),
        );
    }
}

fn draw_achievement_popup(id: &str, alpha: f32) {
    let def = ACHIEVEMENTS.iter().find(|a| a.id == id);
    if let Some(d) = def {
        let w = 320.0;
        let h = 60.0;
        let x = screen_width() / 2.0 - w / 2.0;
        let y = 80.0 + (1.0 - alpha) * 40.0;
        draw_rectangle(x, y, w, h, Color::new(0.0, 0.0, 0.0, alpha * 0.85));
        draw_rectangle_lines(x, y, w, h, 2.0, Color::new(1.0, 0.84, 0.0, alpha));
        draw_text_ex(
            &format!("{} ACHIEVEMENT: {}", d.emoji, d.name),
            x + 10.0,
            y + 35.0,
            TextParams {
                font_size: 20,
                color: Color::new(1.0, 0.9, 0.3, alpha),
                ..Default::default()
            },
        );
    }
}

fn draw_combo(world: &World) {
    if world.combo.count > 1 {
        let c = if world.combo.count > 5 {
            RED
        } else if world.combo.count > 3 {
            Color::new(1.0, 0.5, 0.0, 1.0)
        } else {
            YELLOW
        };
        draw_text_ex(
            &format!("COMBO x{}!", world.combo.count),
            screen_width() / 2.0 - 80.0,
            50.0,
            TextParams {
                font_size: 32,
                color: c,
                ..Default::default()
            },
        );
    }
}

fn draw_transition(alpha: f32) {
    draw_rectangle(
        0.0,
        0.0,
        screen_width(),
        screen_height(),
        Color::new(0.0, 0.0, 0.0, alpha),
    );
    if alpha > 0.3 {
        draw_text_ex(
            &format!("DESCENDING TO FLOOR..."),
            screen_width() / 2.0 - 150.0,
            screen_height() / 2.0,
            TextParams {
                font_size: 28,
                color: Color::new(1.0, 0.8, 0.3, alpha),
                ..Default::default()
            },
        );
    }
}

// ============================================================================
// MAIN
// ============================================================================
struct Game {
    services: Services,
    save: SaveData,
    world: Option<World>,
    scene: Scene,
    font: Font,
    input: InputBuffer,
    achievement_timer: f32,
    achievement_id: Option<String>,
    ambient_started: bool,
}

impl Game {
    async fn new() -> Self {
        let font = load_ttf_font("./res/NotoEmoji-Regular.ttf")
            .await
            .expect("Missing res/NotoEmoji-Regular.ttf");
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let services = Services::new(seed);
        services.audio.borrow_mut().load_samples().await;
        let save = SaveData::load();
        Self {
            services,
            save,
            world: None,
            scene: Scene::MainMenu,
            font,
            input: InputBuffer::new(INPUT_BUFFER_WINDOW),
            achievement_timer: 0.0,
            achievement_id: None,
            ambient_started: false,
        }
    }

    fn start_run(&mut self) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.services.rng = Rc::new(RefCell::new(DeterministicRng::new(seed)));
        self.world = Some(World::new(seed, &self.save));
        self.scene = Scene::Playing;
        self.save.stats.runs += 1;
        self.save.save();
    }

    fn poll_input(&mut self, t: f32) {
        if is_key_pressed(KeyCode::W) || is_key_pressed(KeyCode::Up) {
            self.input.push(InputAction::Up, t);
        }
        if is_key_pressed(KeyCode::S) || is_key_pressed(KeyCode::Down) {
            self.input.push(InputAction::Down, t);
        }
        if is_key_pressed(KeyCode::A) || is_key_pressed(KeyCode::Left) {
            self.input.push(InputAction::Left, t);
        }
        if is_key_pressed(KeyCode::D) || is_key_pressed(KeyCode::Right) {
            self.input.push(InputAction::Right, t);
        }
        if is_key_pressed(KeyCode::Space) {
            self.input.push(InputAction::Wall, t);
        }
        if is_key_pressed(KeyCode::Escape) {
            self.input.push(InputAction::Pause, t);
        }
        if is_key_pressed(KeyCode::E) {
            self.input.push(InputAction::Interact, t);
        }
    }

    fn update_playing(&mut self, dt: f32) {
        let t = self.services.time.borrow().total;
        if let Some(world) = &mut self.world {
            // Movement via command queue
            let move_cd = world.move_cd;
            if move_cd <= 0.0 {
                let mut dir = None;
                if self.input.consume(InputAction::Up, t) {
                    dir = Some(Dir::Up);
                } else if self.input.consume(InputAction::Down, t) {
                    dir = Some(Dir::Down);
                } else if self.input.consume(InputAction::Left, t) {
                    dir = Some(Dir::Left);
                } else if self.input.consume(InputAction::Right, t) {
                    dir = Some(Dir::Right);
                }
                if let Some(d) = dir {
                    world.commands.push(Box::new(CmdMove { dir: d }));
                    let speed = if t < world.speed_until { 0.08 } else { 0.14 };
                    world.move_cd = speed;
                    self.services.audio.borrow().play(&self.services.audio.borrow().footstep);
                }
            }
            // Wall placement
            if world.wall_cd <= 0.0 && self.input.consume(InputAction::Wall, t) {
                let p = world.prev_player;
                world.commands.push(Box::new(CmdPlaceWall { pos: p }));
                world.wall_cd = 0.7;
                self.services.audio.borrow().play(&self.services.audio.borrow().wall);
            }
            // Drain commands
            world.commands.drain(world);

            // Update world
            world.update(dt, &self.services, &self.save);

            // Handle pending achievements
            if let Some(id) = world.pending_achievements.pop() {
                if self.save.unlock(&id) {
                    self.achievement_id = Some(id);
                    self.achievement_timer = 3.0;
                }
            }

            // Death check
            if world.sanity <= 0.0 {
                self.save.add_score(world.floor, world.steps, world.score());
                self.save.stats.time += self.services.time.borrow().total;
                self.save.save();
                self.scene = Scene::GameOver;
            }
        }

        // Pause
        if self.input.consume(InputAction::Pause, t) {
            self.scene = Scene::Paused;
        }
    }

    fn render_playing(&self) {
        if let Some(world) = &self.world {
            draw_grid(world, &self.font);
            self.world.as_ref().unwrap().particles.render(&self.font);
            draw_hud(world, &self.font, &self.save);
            draw_minimap(world);
            draw_combo(world);
            self.weather_render(world);
            draw_crt();
            if world.transitioning {
                draw_transition(world.transition);
            }
            if let Some(id) = &self.achievement_id {
                draw_achievement_popup(id, self.achievement_timer.min(1.0));
            }
        }
    }

    fn weather_render(&self, world: &World) {
        world.weather.render();
    }

    fn render_main_menu(&self) {
        let (w, h) = (450.0, 420.0);
        let (x, y) = ((screen_width() - w) / 2.0, (screen_height() - h) / 2.0);
        root_ui().window(
            hash!(),
            Vec2::new(x, y),
            Vec2::new(w, h),
            |ui| {
                ui.label(None, "THE SMILING DARK");
                ui.label(None, "A S C E N S I O N");
                ui.separator();
                ui.label(None, "WASD / Arrows: Move");
                ui.label(None, "SPACE: Place 🧱 wall");
                ui.label(None, "ESC: Pause");
                ui.label(None, "Find 🪜 to descend deeper");
                ui.label(None, "Avoid 👁️ 👻 💀 🪤");
                ui.separator();
                if ui.button(None, "▶ NEW RUN") {
                    // signal via scene
                }
                if ui.button(None, "🏆 HIGH SCORES") {
                    // signal
                }
                if ui.button(None, "🌳 SKILL TREE") {
                    // signal
                }
                if ui.button(None, "✖ QUIT") {
                    std::process::exit(0);
                }
            },
        );
    }

    fn render_game_over(&self) {
        draw_rectangle(
            0.0,
            0.0,
            screen_width(),
            screen_height(),
            Color::new(0.0, 0.0, 0.0, 0.85),
        );
        let (w, h) = (420.0, 300.0);
        let (x, y) = ((screen_width() - w) / 2.0, (screen_height() - h) / 2.0);
        root_ui().window(
            hash!(),
            Vec2::new(x, y),
            Vec2::new(w, h),
            |ui| {
                ui.label(None, "💀 YOU HAVE PERISHED 💀");
                ui.separator();
                if let Some(world) = &self.world {
                    ui.label(None, &format!("Floor: {}", world.floor));
                    ui.label(None, &format!("Steps: {}", world.steps));
                    ui.label(None, &format!("Score: {}", world.score()));
                    ui.label(None, &format!("Crystals: {}", world.crystals));
                    ui.label(None, &format!("Walls: {}", world.walls_placed));
                }
                ui.separator();
                ui.label(None, "TOP SCORES:");
                for (i, s) in self.save.high_scores.iter().take(3).enumerate() {
                    ui.label(None, &format!("{}. Floor {} - {} pts", i + 1, s.floor, s.score));
                }
                ui.separator();
                if ui.button(None, "↻ TRY AGAIN") {
                    // signal
                }
                if ui.button(None, "🏠 MAIN MENU") {
                    // signal
                }
            },
        );
    }

    fn render_paused(&self) {
        draw_rectangle(
            0.0,
            0.0,
            screen_width(),
            screen_height(),
            Color::new(0.0, 0.0, 0.0, 0.7),
        );
        let (w, h) = (300.0, 200.0);
        let (x, y) = ((screen_width() - w) / 2.0, (screen_height() - h) / 2.0);
        root_ui().window(
            hash!(),
            Vec2::new(x, y),
            Vec2::new(w, h),
            |ui| {
                ui.label(None, "PAUSED");
                ui.separator();
                if ui.button(None, "▶ RESUME") {
                    // signal
                }
                if ui.button(None, "🏠 QUIT TO MENU") {
                    // signal
                }
            },
        );
    }
}

#[macroquad::main("The Smiling Dark: Ascension")]
async fn main() {
    request_new_screen_size(1000.0, 720.0);
    let mut game = Game::new().await;

    let mut last = get_time();
    // Track button clicks via UI state
    let mut pending_action: Option<&str> = None;

    loop {
        let now = get_time();
        let dt = (now - last) as f32;
        last = now;

        // Time update
        game.services.time.borrow_mut().update(dt);
        let t = game.services.time.borrow().total;

        // Poll input
        game.poll_input(t);

        // Dispatch events
        game.services.events.borrow_mut().dispatch();

        // Achievement popup timer
        if game.achievement_timer > 0.0 {
            game.achievement_timer -= dt;
            if game.achievement_timer <= 0.0 {
                game.achievement_id = None;
            }
        }

        clear_background(Color::new(0.02, 0.02, 0.04, 1.0));

        match game.scene {
            Scene::MainMenu => {
                game.render_main_menu();
                // Check button clicks via UI state is tricky; use key shortcuts
                if is_key_pressed(KeyCode::Enter) {
                    game.start_run();
                    if !game.ambient_started {
                        game.services.audio.borrow().play_ambient();
                        game.ambient_started = true;
                    }
                }
                if is_key_pressed(KeyCode::H) {
                    game.scene = Scene::HighScores;
                }
                if is_key_pressed(KeyCode::K) {
                    game.scene = Scene::SkillTree;
                }
            }
            Scene::Playing => {
                game.update_playing(dt);
                game.render_playing();
            }
            Scene::Paused => {
                if let Some(w) = &game.world {
                    game.render_playing();
                }
                game.render_paused();
                if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Enter) {
                    game.scene = Scene::Playing;
                }
                if is_key_pressed(KeyCode::Q) {
                    game.scene = Scene::MainMenu;
                    game.world = None;
                }
            }
            Scene::HighScores => {
                let (w, h) = (400.0, 400.0);
                let (x, y) = ((screen_width() - w) / 2.0, (screen_height() - h) / 2.0);
                root_ui().window(
                    hash!(),
                    Vec2::new(x, y),
                    Vec2::new(w, h),
                    |ui| {
                        ui.label(None, "🏆 HIGH SCORES");
                        ui.separator();
                        if game.save.high_scores.is_empty() {
                            ui.label(None, "No scores yet. Go play!");
                        } else {
                            for (i, s) in game.save.high_scores.iter().enumerate() {
                                ui.label(
                                    None,
                                    &format!("{}. Floor {} | {} steps | {} pts", i + 1, s.floor, s.steps, s.score),
                                );
                            }
                        }
                        ui.separator();
                        if ui.button(None, "◀ BACK") {
                            // signal
                        }
                    },
                );
                if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Enter) {
                    game.scene = Scene::MainMenu;
                }
            }
            Scene::SkillTree => {
                let (w, h) = (450.0, 400.0);
                let (x, y) = ((screen_width() - w) / 2.0, (screen_height() - h) / 2.0);
                root_ui().window(
                    hash!(),
                    Vec2::new(x, y),
                    Vec2::new(w, h),
                    |ui| {
                        ui.label(None, &format!("🌳 SKILL TREE (Points: {})", game.save.skill_points));
                        ui.separator();
                        let skills = [
                            ("tough_mind", "+20 starting sanity"),
                            ("swift_feet", "10% faster movement"),
                            ("eagle_eye", "20% larger vision"),
                            ("lucky", "More item spawns"),
                        ];
                        for (id, desc) in skills.iter() {
                            let owned = game.save.has_skill(id);
                            let label = if owned {
                                format!("✓ {} - {}", id, desc)
                            } else {
                                format!("  {} - {} (1 pt)", id, desc)
                            };
                            ui.label(None, &label);
                            if !owned && ui.button(None, &format!("Learn {}", id)) {
                                game.save.learn_skill(id);
                            }
                        }
                        ui.separator();
                        if ui.button(None, "◀ BACK") {
                            // signal
                        }
                    },
                );
                if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Enter) {
                    game.scene = Scene::MainMenu;
                }
            }
            Scene::GameOver => {
                game.render_game_over();
                if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::R) {
                    game.start_run();
                }
                if is_key_pressed(KeyCode::Escape) || is_key_pressed(KeyCode::Q) {
                    game.scene = Scene::MainMenu;
                    game.world = None;
                }
            }
            Scene::Victory => {
                // Reserved
                game.scene = Scene::MainMenu;
            }
        }

        next_frame().await;
    }
}