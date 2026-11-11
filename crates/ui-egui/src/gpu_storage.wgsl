// What `gpu.wgsl` reads, from storage buffers and the opaque depth texture: Vulkan, Metal, DirectX 12 and WebGPU.

@group(0) @binding(1) var<storage, read> owners: array<Owner>;
@group(0) @binding(2) var<storage, read> segs: array<Seg>;
@group(0) @binding(3) var<storage, read> skins: array<Skin>;
@group(0) @binding(4) var<storage, read> texels: array<u32>;
// Each copy's triangles' plane numbers, from where its instance says.
@group(0) @binding(5) var<storage, read> planes: array<u32>;

@group(1) @binding(2) var scene_depth: texture_depth_2d;

// The plane a pixel shows and its element; the depth is the depth target's.
alias Ids = vec2<u32>;

fn pack_ids(ids: vec2<u32>, depth: f32) -> Ids {
    return ids;
}

fn owner_at(i: u32) -> Owner {
    return owners[i];
}

fn skin_at(i: u32) -> Skin {
    return skins[i];
}

fn texel_word(i: u32) -> u32 {
    return texels[i];
}

fn plane_at(i: u32) -> u32 {
    return planes[i];
}

fn opaque_depth(q: vec2<i32>) -> f32 {
    return textureLoad(scene_depth, q, 0);
}

@vertex
fn vs_line(@builtin(vertex_index) i: u32, @builtin(instance_index) k: u32) -> LineOut {
    return line_out(segs[k], i);
}
