// What `gpu.wgsl` reads, for a card with no storage buffers and no depth texture a shader can read: WebGL2 (and
// OpenGL ES 3.0), where the 3D view would otherwise be the processor's. The tables are 2D textures of four 32-bit words
// a texel, `ROW` texels to a row (`ROW` is written before this file, as wide as the card's textures go), read with
// `textureLoad`, which takes no sampler and filters nothing; the levels' lines are an instance vertex buffer; and the
// opaque surfaces' depth is written in the id target's third word as well as the depth target, which only tests it.

@group(0) @binding(1) var owner_table: texture_2d<f32>;
@group(0) @binding(3) var skin_table: texture_2d<u32>;
@group(0) @binding(4) var texel_table: texture_2d<u32>;
@group(0) @binding(5) var plane_table: texture_2d<u32>;

// The plane a pixel shows, its element, and its depth's bits.
alias Ids = vec4<u32>;

fn pack_ids(ids: vec2<u32>, depth: f32) -> Ids {
    return vec4<u32>(ids, bitcast<u32>(depth), 0u);
}

// Texel `k` of a table.
fn cell(k: u32) -> vec2<i32> {
    return vec2<i32>(i32(k % ROW), i32(k / ROW));
}

// Two texels: the edge and the tint.
fn owner_at(i: u32) -> Owner {
    var o: Owner;
    o.edge = textureLoad(owner_table, cell(2u * i), 0);
    o.tint = textureLoad(owner_table, cell(2u * i + 1u), 0);
    return o;
}

// Three texels: size and turn, offset and spare (floats, by their bits), and the level.
fn skin_at(i: u32) -> Skin {
    let a = textureLoad(skin_table, cell(3u * i), 0);
    let b = textureLoad(skin_table, cell(3u * i + 1u), 0);
    var s: Skin;
    s.size = bitcast<vec2<f32>>(a.xy);
    s.turn = bitcast<vec2<f32>>(a.zw);
    s.offset = bitcast<vec2<f32>>(b.xy);
    s.spare = bitcast<vec2<f32>>(b.zw);
    s.level = textureLoad(skin_table, cell(3u * i + 2u), 0);
    return s;
}

// Word `i`, four to a texel.
fn texel_word(i: u32) -> u32 {
    let t = textureLoad(texel_table, cell(i / 4u), 0);
    return t[i % 4u];
}

fn plane_at(i: u32) -> u32 {
    let t = textureLoad(plane_table, cell(i / 4u), 0);
    return t[i % 4u];
}

fn opaque_depth(q: vec2<i32>) -> f32 {
    return bitcast<f32>(textureLoad(scene_ids, q, 0).z);
}

// A segment's ends (w: how many px wide) and colour, an instance each.
struct SegIn {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_line(v: SegIn, @builtin(vertex_index) i: u32) -> LineOut {
    var s: Seg;
    s.a = v.a;
    s.b = v.b;
    s.color = v.color;
    return line_out(s, i);
}
