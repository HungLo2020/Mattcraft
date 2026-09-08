//! Actual Vulkan reproduction of 16/32px sprites at independently captured GUI scales.
//! This is an isolated raster-stage test, not a whole-game parity admission.
use super::*;
use crate::render::vulkanic::backends::vulkan::VulkanBackend;
use crate::render::vulkanic::gui_item_material::{GuiAffineMaterial, GuiFlatItemLighting};
use crate::render::vulkanic::gui_item_raster::{GuiItemRasterLayout, GuiItemRasterRows, GuiItemRasterTarget};
use crate::render::vulkanic::world_primitive_frontend::{WorldPrimitiveFrontend,
    WorldMeshTextureAssetPayload, WORLD_MATERIAL_TEXTURE_STONE};

const ITEM_TINTS: [u32; 9] = [0xFFFF8040, 0xFF40FF80, 0xFF8040FF,
    0xFFFFFFFF, 0xFF80FFFF, 0xFFFF80FF, 0xFFFFFF80, 0xFF808080, 0xFF4080C0];

const ITEM_RECTS: [[usize;4];9] = [[0,0,8,8], [8,0,16,8], [0,8,8,16],
    [8,8,16,16], [4,4,12,12], [0,4,16,12], [4,0,12,16], [2,6,10,14], [0,0,16,16]];
const ITEM_ORIENTATIONS: [[i64;6];8] = [[0,0,16,0,0,16], [16,0,0,0,16,16],
    [0,16,16,16,0,0], [16,16,0,16,16,0], [0,0,0,16,16,0],
    [16,0,16,16,0,0], [0,16,0,0,16,16], [16,16,16,0,0,16]];

// Independent Frozen OpenGL r258 center pixels, in ITEM_TINTS order.
// The CPU-only unquantized formula rounds green212 *252/255 to210, while
// Frozen's actual RGBA8 output is209, including the existing untinted case.
// Keep an exact baseline oracle; do not widen the pixel assertion to hide it.
const FROZEN_TINTED_CENTERS: [[u8;3];9] = [[35,105,24], [9,209,49], [17,53,97],
    [35,209,97], [17,209,97], [35,105,97], [35,209,49], [17,105,49], [9,105,73]];

fn tinted_item_pixel(column: usize, row: usize, size: usize, cutout: bool,
    transparency: bool, tint: u32) -> [u8; 4] {
    let alpha_range = size/8..3*size/8;
    let (source, alpha) = if transparency && alpha_range.contains(&row) && alpha_range.contains(&column) {
        if column < size/4 { return [0;4]; }
        ([255.0;3], 26_u8)
    } else if row == 0 { ([0.0;3], 255) }
    else if column == 0 { ([255.0,255.0,0.0], 255) }
    else if row == size-1 || column == size-1 { ([17.0,106.0,49.0], 255) }
    else {
        let index = ITEM_TINTS.iter().position(|candidate| *candidate == tint).unwrap();
        let [r,g,b] = FROZEN_TINTED_CENTERS[index];
        return [r,g,b,255];
    };
    let mut output = [0,0,0,alpha];
    for (channel, shift) in [16,8,0].into_iter().enumerate() {
        let lit = source[channel] * f64::from((tint >> shift) & 255) / 255.0 * 252.0 / 255.0;
        let raster = (lit * if cutout { 1.0 } else { f64::from(alpha)/255.0 }).round();
        output[channel] = (raster * f64::from(alpha)/255.0).round() as u8;
    }
    output
}

#[test]
fn vulkan_item_raster_uses_owned_atlas_at_observed_scale_and_retires_all_resources() {
    for north_face in [false,true] {
        for bottom_up in [false,true] { raster_and_composite(north_face, bottom_up, 0, false, 3); }
    }
    raster_and_composite(false,true,9,false,3);
}

#[test]
fn vulkan_item_raster_applies_frozen_translucent_cutoff_before_composition() {
    raster_and_composite(false, true, 0, true, 3);
}

#[test]
fn vulkan_item_raster_scale_two_preserves_cutoff_composition_and_bounded_residency() {
    raster_and_composite(false, true, 0, true, 2);
    raster_and_composite(false, true, 9, true, 2);
}

#[test]
fn vulkan_item_raster_scale_one_preserves_downsampled_edges_and_alpha() {
    raster_and_composite(false, true, 0, true, 1);
    raster_and_composite(false, true, 9, true, 1);
}

fn raster_and_composite(north_face: bool, bottom_up: bool, retained_slots:u32, transparency:bool, scale:u32) {
    raster_and_composite_size(north_face, bottom_up, retained_slots, transparency, scale, 32);
}

#[test]
fn vulkan_native_size_item_atlas_preserves_texels_alpha_and_reload_lifetime() {
    for scale in 1..=3 {
        raster_and_composite_size(false, true, 0, true, scale, 16);
    }
}

fn raster_and_composite_size(north_face: bool, bottom_up: bool, retained_slots:u32,
    transparency:bool, scale:u32, sprite_size:usize) {
    raster_and_composite_animation(north_face, bottom_up, retained_slots, transparency, scale, sprite_size, false);
}

#[test]
fn vulkan_gui_only_items_rerasterize_every_interpolated_atlas_tick_without_binding_growth() {
    raster_and_composite_animation(false, true, 0, false, 2, 32, true);
}

fn raster_and_composite_animation(north_face: bool, bottom_up: bool, retained_slots:u32,
    transparency:bool, scale:u32, sprite_size:usize, interpolate:bool) {
    let alpha_range = sprite_size / 8..3 * sprite_size / 8;
    let alpha_split = sprite_size / 4;
    let cell = (16 * scale) as usize;
    let columns = 512 / cell;
    let backend = VulkanBackend::new("GUI item raster stage regression")
        .expect("Vulkan is required for the item raster regression");
    let mut gal = VulkanicGal::new_with_backend(Box::new(backend), false);
    let mut world = WorldPrimitiveFrontend::default();
    let mut frontend = GuiFrontend::default();
    // Positions and ordering independently recorded from both callsites in
    // r226. Adjacent atlas texels are deliberately magenta, never clamped by
    // the test to conceal an out-of-region sample.
    let regions = [(1536,1904), (1568,1776), (1536,1840), (1568,1648),
        (1568,1872), (1568,1808), (1536,1744), (1536,1776), (1536,1712)];
    let mut pixels = [255, 0, 255, 255].repeat(4096 * 2048);
    for (x, y) in regions {
        for row in 0..sprite_size {
            for column in 0..sprite_size {
                let value = if transparency && alpha_range.contains(&row) && alpha_range.contains(&column) {
                    // Frozen's item fragment shader uses strictly alpha < 0.1.
                    // Nearest RGBA8 values immediately below and above it.
                    [255,255,255, if column < alpha_split {25} else {26}]
                } else if row == 0 { [0,0,0,255] }
                    else if column == 0 { [255,255,0,255] }
                    else if row == sprite_size-1 || column == sprite_size-1 { [17,106,49,255] }
                    else { [35,212,98,255] };
                let offset = ((y + row) * 4096 + x + column) * 4;
                pixels[offset..offset+4].copy_from_slice(&value);
            }
        }
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 4096, 2048);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    drop(pixels);
    world.apply_world_mesh_asset_update(&mut gal, 1, vec![], vec![WorldMeshTextureAssetPayload {
        texture_id: WORLD_MATERIAL_TEXTURE_STONE, png_bytes: encoded, mip_png_bytes: vec![],
        frame_width: 0, frame_height: 0, frame_count: 1, frame_ticks: 1, animation_flags: 0,
        frame_row_size: 0, interpolation_policy: 0, animation_frames: vec![],
        coordinate_origin: 0, sampling: None, requested_mip_levels: 1,
    }]).unwrap();
    let incarnation = world.accepted_gui_atlas_incarnation(WORLD_MATERIAL_TEXTURE_STONE).unwrap();
    let references: Vec<_> = regions.iter().enumerate().map(|(index, &(x,y))| GuiAtlasReference {
        asset_id: index as u64 + 1, atlas: incarnation, x: x as u32, y: y as u32,
        width: sprite_size as u32, height: sprite_size as u32,
    }).collect();
    frontend.stage_owned_atlas_references(&mut gal, &world, 1, &references).unwrap();
    let extent = Extent3d { width: 512, height: 512, depth: 1 };
    let creates = gal.metrics().resource_creates;
    assert!(GuiItemRasterTarget::create(&mut gal, Extent3d {width:4097,height:512,depth:1}).is_err());
    assert_eq!(creates,gal.metrics().resource_creates);
    let raster = GuiItemRasterTarget::create(&mut gal,extent).unwrap();
    assert_eq!(raster.extent,extent);
    let (color,view,target,pass) = (raster.color,raster.view,raster.target,raster.pass);
    let readback = gal.create_buffer(BufferDesc { label: "item-raster.readback".into(),
        size: 512*512*4, memory: MemoryDomain::Readback,
        usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead] }).unwrap();
    let layout = GuiItemRasterLayout::new(scale, 9+retained_slots, 4096).unwrap();
    let requests: Vec<_> = (0..9).map(|index| {
        let placement = layout.placement_with_rows(index+retained_slots,
            if bottom_up {GuiItemRasterRows::BottomUp} else {GuiItemRasterRows::TopDown}).unwrap();
        (placement, GuiAffineQuadRequest {
        item_raster_layers: vec![],
        item_raster_scale: 0,
        item_raster_geometry: Default::default(),
        material: {
            let lighting = GuiFlatItemLighting {lightmap_generation: 1, rgb: [252.0/255.0;3]};
            if transparency && index == 6 { GuiAffineMaterial::FlatItemCutout(lighting) }
            else { GuiAffineMaterial::FlatItem(lighting) }
        },
        stratum: 1, asset_id: index as u64 + 1,
        x0: if north_face {16.0} else {0.0}, y0: 0.0,
        x1: if north_face {0.0} else {16.0}, y1: 0.0,
        x3: if north_face {16.0} else {0.0}, y3: 16.0,
        z: 0.0, u0: if north_face {1.0} else {0.0}, v0: 0.0,
        u1: if north_face {0.0} else {1.0}, v1: 1.0, color_argb: 0xffffffff,
        gui_width: 16, gui_height: 16, projection_extent: [16.0;2], sequence: index as u64 + 1,
        clip_mode: 0, clip_left: 0, clip_top: 0, clip_width: 0, clip_height: 0,
    })}).collect();
    // Malformed placement rejects before creating any draw resources.
    let mut invalid = requests.clone();
    invalid[0].0.target_extent = [256,256];
    let creates = gal.metrics().resource_creates;
    assert!(frontend.append_owned_item_raster_quads(&mut gal, &mut world, pass, target, view, &invalid).is_err());
    assert_eq!(creates, gal.metrics().resource_creates);
    for invalid_uv in [f32::NAN, f32::INFINITY, -2.0, 2.0] {
        let mut invalid = requests.clone();
        invalid[8].1.u0 = invalid_uv;
        assert!(frontend.append_owned_item_raster_quads(&mut gal, &mut world, pass, target, view, &invalid).is_err());
        assert_eq!(creates, gal.metrics().resource_creates,
            "oriented endpoints retain the same finite/range validation before allocation");
    }
    let mut ops = vec![CommandOp::Barrier(texture_barrier(color, TextureUsageState::Undefined,
        TextureUsageState::ColorAttachment)), CommandOp::BeginPass { pass, target,
        colors: vec![PassAttachment { view, load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            clear_color: Some(crate::render::vulkanic::commands::ClearColor { r:0.0,g:0.0,b:0.0,a:0.0 }) }],
        depth_stencil: None }, CommandOp::EndPass];
    ops.extend(frontend.append_owned_item_raster_quads(&mut gal, &mut world, pass, target, view, &requests).unwrap());
    assert!(frontend.raw_images.is_empty(), "the raster stage must sample the world-owned atlas");
    ops.extend([CommandOp::Barrier(texture_barrier(color, TextureUsageState::ColorAttachment,
        TextureUsageState::TransferSrc)), CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
        buffer: readback, buffer_offset: 0, bytes_per_row: 512*4, rows_per_image: 512,
        texture: color, texture_mip: 0, texture_layer: 0,
        texture_origin: TextureOrigin3d { x:0,y:0,z:0 }, extent }),
        CommandOp::Barrier(buffer_barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead)),
        CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 512*512*4 }]);
    let list = gal.create_command_list(CommandListDesc { label: "item-raster.commands".into(), operations: ops }).unwrap();
    let token = gal.submit(SubmissionBatch { label: "item-raster.submit".into(), command_lists: vec![list] }).unwrap();
    gal.retire_through_for_test(token.submission).unwrap();
    let reads = gal.completed_host_reads();
    let bytes = &reads.iter().find(|r| r.buffer == readback).unwrap().bytes;
    let pixel = |x:usize,y:usize| {
        let slot=x/cell+retained_slots as usize;
        let x=slot%columns*cell+x%cell;
        let y=slot/columns*cell+y;
        let y = if bottom_up {511-y} else {y};
        &bytes[(y*512+x)*4..(y*512+x+1)*4]
    };
    for index in 0..9 {
        let x = index*cell;
        // The independent Frozen r226 capture records this same lit center.
        assert_eq!(pixel(x+cell/2,cell/2), [35,209,97,255]);
        // Frozen's captured 32px scale-one icons select source texel one at
        // the first destination pixel; native 16px sprites retain that border.
        assert_eq!(pixel(x+cell/2,0), if scale == 1 && sprite_size == 32 {[35,209,97,255]} else {[0,0,0,255]});
        assert_eq!(pixel(x,cell/2), if scale == 1 && sprite_size == 32 {[35,209,97,255]} else {[252,252,0,255]});
        assert_eq!(pixel(x+cell-1,cell/2), [17,105,48,255]);
        if transparency {
            assert_eq!(pixel(x+3*scale as usize,3*scale as usize), [0,0,0,0], "25/255 must be discarded before blending");
            assert_eq!(pixel(x+5*scale as usize,3*scale as usize), if index == 6 {[252,252,252,26]} else {[26,26,26,26]},
                "26/255 survives; cutout writes unblended RGB, translucent uses source alpha");
        }
        let rows: Vec<_> = (0..cell).filter(|&y| y == 0 || pixel(x+cell/2,y) != pixel(x+cell/2,y-1))
            .map(|y| (y,pixel(x+cell/2,y).to_vec())).collect();
        let transitions: Vec<_> = rows.iter().map(|(row,_)| *row).collect();
        // Native sprites map texels to integral scale-sized blocks. For 32px,
        // scale two is 1:1; scale three preserves captured half-texel tie cases.
        let expected = if sprite_size == 16 {vec![0,scale as usize,15*scale as usize]}
        else if scale == 1 {vec![0,15]} else if scale == 2 {vec![0,1,31]} else if !bottom_up {vec![0,2,47]} else if retained_slots==9 && ![0,5,7].contains(&index) {
            vec![0,2,46]
        } else {vec![0,1,46]};
        assert_eq!(transitions, expected,
            "native texel blocks and Frozen 32px captures establish these row transitions");
        eprintln!("item-raster north={north_face} bottom-up={bottom_up} retained={retained_slots} index={index} center-column-transitions={rows:?}");
    }
    assert_eq!(pixel(9*cell,cell/2), [0,0,0,0]);
    if scale == 3 { assert_eq!(pixel(24,48), [0,0,0,0]); }
    // The last cell's following row is outside every occupied cell.
    assert_eq!(pixel(8*cell+cell/2,cell), [0,0,0,0]);
    // The second submission samples the GPU raster directly. Moving cells to
    // a different row ensures accidental source-as-destination or a missing
    // composition draw cannot satisfy the assertions.
    let output = gal.create_texture(TextureDesc { label: "item-composite.color".into(),
        dimension: TextureDimension::D2, format: TextureFormat::Rgba8Unorm, extent,
        mip_levels: 1, array_layers: 1,
        usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc] }).unwrap();
    let output_view = gal.create_texture_view(TextureViewDesc { label: "item-composite.view".into(),
        texture: output, format: TextureFormat::Rgba8Unorm, base_mip: 0, mip_count: 1,
        base_layer: 0, layer_count: 1 }).unwrap();
    let output_target = gal.create_render_target(RenderTargetDesc { label: "item-composite.target".into(),
        color_views: vec![output_view], depth_stencil_view: None, extent }).unwrap();
    let output_pass = gal.create_render_pass(RenderPassDesc { label: "item-composite.pass".into(),
        target: output_target, color_formats: vec![TextureFormat::Rgba8Unorm], depth_format: None }).unwrap();
    let composite = GuiMeshCompositeResources::create(&mut gal, "item-composite",
        ColorFormat::Rgba8Unorm, None, view).unwrap();
    let mut ops = vec![CommandOp::Barrier(texture_barrier(output, TextureUsageState::Undefined,
        TextureUsageState::ColorAttachment)), CommandOp::BeginPass { pass: output_pass, target: output_target,
        colors: vec![PassAttachment { view: output_view, load_op: AttachmentLoadOp::Clear,
            store_op: AttachmentStoreOp::Store,
            clear_color: Some(crate::render::vulkanic::commands::ClearColor { r:0.0,g:0.0,b:0.0,a:0.0 }) }],
        depth_stencil: None }, CommandOp::EndPass];
    for (index,(placement,local)) in requests.iter().enumerate() {
        let mut screen = local.clone();
        screen.x0 = index as f32 * cell as f32;
        screen.x1 = screen.x0 + cell as f32;
        screen.x3 = screen.x0;
        screen.y0 = 100.0; screen.y1 = 100.0; screen.y3 = 100.0 + cell as f32;
        screen.projection_extent = [512.0;2];
        composite.append_item_raster_composite(color,
            if index == 0 { TextureUsageState::TransferSrc } else { TextureUsageState::ShaderRead },
            *placement, output_pass, output_target, output_view, None, &screen,
            false,index as u64 * GUI_MESH_COMPOSITE_UNIFORM_STRIDE, &mut ops).unwrap();
    }
    ops.extend([CommandOp::Barrier(texture_barrier(output, TextureUsageState::ColorAttachment,
        TextureUsageState::TransferSrc)),
        CommandOp::Barrier(buffer_barrier(readback, TextureUsageState::ShaderRead, TextureUsageState::TransferDst)),
        CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {
        buffer: readback, buffer_offset: 0, bytes_per_row: 512*4, rows_per_image: 512,
        texture: output, texture_mip: 0, texture_layer: 0,
        texture_origin: TextureOrigin3d { x:0,y:0,z:0 }, extent }),
        CommandOp::Barrier(buffer_barrier(readback, TextureUsageState::TransferDst, TextureUsageState::ShaderRead)),
        CommandOp::HostReadBuffer { buffer: readback, offset: 0, size: 512*512*4 }]);
    let list = gal.create_command_list(CommandListDesc { label: "item-composite.commands".into(), operations: ops }).unwrap();
    let token = gal.submit(SubmissionBatch { label: "item-composite.submit".into(), command_lists: vec![list] }).unwrap();
    gal.retire_through_for_test(token.submission).unwrap();
    let composite_reads = gal.completed_host_reads();
    let output_bytes = &composite_reads.iter().rev().find(|r| r.buffer == readback).unwrap().bytes;
    for y in 0..cell {
        for x in 0..9*cell {
            let source = pixel(x,y);
            let expected = if source[3] == 26 {
                if source[0] == 252 {[26,26,26,26]} else {[3,3,3,26]}
            } else {
                [source[0],source[1],source[2],source[3]]
            };
            assert_eq!(&output_bytes[((y+100)*512+x)*4..((y+100)*512+x+1)*4], expected,
                "composition uses the explicit source-alpha blend at {x},{y}");
        }
    }
    assert_eq!(&output_bytes[0..4], [0,0,0,0]);
    if bottom_up && !north_face && retained_slots == 0 && scale == 2 && !interpolate {
        // Two translucent authored layers MUST blend into one item image
        // before the image is composited once. Alternate ordering distinguishes
        // whole-item composition from independently compositing child layers.
        let groups: Vec<_> = requests.iter().enumerate().map(|(index,(_,local))| {
            let mut presentation = local.clone();
            presentation.item_raster_scale=2;
            presentation.x0=index as f32*32.0; presentation.x1=presentation.x0+32.0;
            presentation.x3=presentation.x0;
            presentation.y0=100.0; presentation.y1=100.0; presentation.y3=132.0;
            presentation.gui_width=512; presentation.gui_height=512;
            presentation.projection_extent=[512.0;2];
            let colors = if index % 2 == 0 {[0x80ff0000,0x8000ff00]} else {[0x8000ff00,0x80ff0000]};
            GuiItemRasterGroup {presentation,layers:colors.into_iter().map(|color_argb|
                GuiItemRasterLayer {asset_id:1,color_argb,
                    material:GuiAffineMaterial::FlatItem(GuiFlatItemLighting {lightmap_generation:1,rgb:[1.0;3]}),
                    geometry:Default::default(),uv:[0.5,0.5,0.75,0.75],model_transform:Default::default()}).collect()}
        }).collect();
        for defect in 0..9 {
            let mut invalid=groups.clone();
            match defect {
                0 => invalid[8].layers.clear(),
                1 => invalid[8].layers=vec![invalid[8].layers[0].clone();65],
                2 => invalid[8].layers[1].uv[2]=f32::NAN,
                3 => invalid[8].layers[1].asset_id=u64::MAX,
                4 => invalid[8].presentation.sequence=invalid[0].presentation.sequence,
                5 => invalid[8].layers[1].material=GuiAffineMaterial::Unlit,
                6 => invalid[8].layers[1].material=GuiAffineMaterial::FlatItem(
                    GuiFlatItemLighting {lightmap_generation:1,rgb:[f32::NAN;3]}),
                7 => invalid[8].layers[1].model_transform.0[3]=0.25,
                _ => invalid[8].layers[1].model_transform.0[12]=5.0,
            }
            let creates=gal.metrics().resource_creates;
            let slots=frontend.item_raster_slots.clone();
            let mut ops=Vec::new();
            assert!(frontend.prepare_item_raster_groups(&mut gal,&mut world,&invalid,
                ColorFormat::Rgba8Unorm,None,&mut GuiSubmitStats::default(),&mut ops).is_err());
            assert!(ops.is_empty(),"malformed final layer must not partially encode earlier items");
            assert_eq!(gal.metrics().resource_creates,creates);
            assert_eq!(frontend.item_raster_slots,slots);
        }
        let mut stable_live = None;
        for repeat in 0..12 {
            if repeat%3==0 { stable_live=None; }
            let creates = gal.metrics().resource_creates;
            let transported=groups.iter().map(|group| {
                let mut request=group.presentation.clone();
                request.item_raster_layers=group.layers.clone();
                if (3..6).contains(&repeat) {
                    for layer in &mut request.item_raster_layers {
                        layer.model_transform.0[0]=0.5;
                        layer.model_transform.0[5]=0.5;
                        layer.model_transform.0[12]=-0.25;
                        layer.model_transform.0[13]=-0.25;
                    }
                }
                if (6..9).contains(&repeat) {
                    // Change only the child matrix, keeping the same resource,
                    // geometry, material, parent transform, and presentation.
                    // A parent-only cache key must not reuse the previous image.
                    let child=&mut request.item_raster_layers[1];
                    child.model_transform.0[0]=0.5;
                    child.model_transform.0[5]=0.5;
                    child.model_transform.0[12]=-0.375;
                    child.model_transform.0[13]=-0.25;
                }
                request
            }).collect();
            let (frame_ops,stats)=frontend.append_frame_ops_with_owned_atlases_to_target(
                &mut gal,Some(&mut world),1,output_target,output_view,Some(output_pass),None,None,false,
                vec![],transported,vec![],vec![]).unwrap();
            assert_eq!(stats.affine_quad_count,9,"exactly one presentation per item, never per child layer");
            assert_eq!(stats.owned_intermediate_targets.len(),1);
            if repeat > 0 && repeat!=3 && repeat!=6 { assert_eq!(gal.metrics().resource_creates,creates); }
            let mut ops=vec![CommandOp::Barrier(texture_barrier(output,TextureUsageState::TransferSrc,
                TextureUsageState::ColorAttachment)),CommandOp::BeginPass {pass:output_pass,target:output_target,
                colors:vec![PassAttachment {view:output_view,load_op:AttachmentLoadOp::Clear,store_op:AttachmentStoreOp::Store,
                    clear_color:Some(crate::render::vulkanic::commands::ClearColor {r:0.0,g:0.0,b:0.0,a:0.0})}],depth_stencil:None},
                CommandOp::EndPass];
            ops.extend(frame_ops);
            ops.extend([CommandOp::Barrier(texture_barrier(output,TextureUsageState::ColorAttachment,TextureUsageState::TransferSrc)),
                CommandOp::Barrier(buffer_barrier(readback,TextureUsageState::ShaderRead,TextureUsageState::TransferDst)),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {buffer:readback,buffer_offset:0,bytes_per_row:512*4,
                    rows_per_image:512,texture:output,texture_mip:0,texture_layer:0,texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent}),
                CommandOp::Barrier(buffer_barrier(readback,TextureUsageState::TransferDst,TextureUsageState::ShaderRead)),
                CommandOp::HostReadBuffer {buffer:readback,offset:0,size:512*512*4}]);
            let list=gal.create_command_list(CommandListDesc {label:"grouped-item.commands".into(),operations:ops}).unwrap();
            let token=gal.submit(SubmissionBatch {label:"grouped-item.submit".into(),command_lists:vec![list]}).unwrap();
            gal.retire_through_for_test(token.submission).unwrap();
            let reads=gal.completed_host_reads();
            let actual=&reads.iter().rev().find(|read|read.buffer==readback).unwrap().bytes;
            for y in 0..512 { for x in 0..512 {
                let covered=if !(3..6).contains(&repeat) { (100..132).contains(&y) }
                    else { (108..124).contains(&y) && (8..24).contains(&(x%32)) };
                let expected=if covered && x<9*32 {
                    if (6..9).contains(&repeat) && !((108..124).contains(&y) && (4..20).contains(&(x%32))) {
                        // Authored texel [35,212,98], alpha128, two RGBA8
                        // stages: round(round(channel*128/255)*128/255).
                        if (x/32)%2 == 0 {[9,0,0,128]} else {[0,53,0,128]}
                    } else if (x/32)%2 == 0 {[7,80,0,192]} else {[14,40,0,192]}
                } else {[0;4]};
                assert_eq!(&actual[(y*512+x)*4..(y*512+x+1)*4],expected,
                    "ordered layers composed once at {x},{y}, repeat{repeat}");
            } }
            let live=gal.metrics().resource_creates-gal.metrics().resource_destroys;
            if let Some(previous)=stable_live { assert_eq!(live,previous); }
            stable_live=Some(live);
        }
    }
    if bottom_up && !north_face && retained_slots==0 {
        let mut previous_live = None;
        let frame_scales = if scale == 3 { vec![3,3,1,2,3,2,1,3] } else { vec![scale,scale] };
        // Exercise the same explicit GUI-generation invalidation used by a
        // resource reload, then a reuse frame for each rebuilt generation.
        // Real world-atlas replacement is independently paired with Frozen;
        // here every rebuilt intermediate must preserve pixels and residency.
        let lifecycle_frames = frame_scales.into_iter().map(|value| (value, 1))
            .chain((2..=5).flat_map(|generation| [(scale, generation), (scale, generation)]))
            .chain((6..=21).flat_map(|generation| [(2, generation), (2, generation)]))
            .chain(std::iter::repeat_n((2,22), if interpolate {8} else {4}));
        let mut animation_tick = 0;
        for (frame_scale, generation) in lifecycle_frames {
            let tinted = (6..22).contains(&generation);
            if generation == 22 {
                use crate::render::vulkanic::sprite_interpolation::{OwnedAtlasAnimationUpdate,
                    OwnedSpriteAnimation, SpriteAnimationClock, SpriteAnimationFrame,
                    SpriteAtlasRegion, SpriteMipSheet, AtlasAnimationTickEvent};
                if animation_tick == 0 {
                    previous_live = None;
                    world.stage_atlas_animation_assets(OwnedAtlasAnimationUpdate {
                        texture_id: WORLD_MATERIAL_TEXTURE_STONE, generation: 1,
                        sprites: regions.iter().enumerate().map(|(index,&(x,y))| {
                            let mut rgba = Vec::new();
                            for _ in 0..sprite_size {
                                rgba.extend([0,0,0,255].repeat(sprite_size));
                                rgba.extend([255;4].repeat(sprite_size));
                            }
                            OwnedSpriteAnimation { sprite_id:index as u32+1,
                                region:SpriteAtlasRegion {x:x as u32,y:y as u32,
                                    width:sprite_size as u32,height:sprite_size as u32},
                                clock:SpriteAnimationClock::new(vec![
                                    SpriteAnimationFrame {index:0,duration_ticks:if interpolate {4} else {1}},
                                    SpriteAnimationFrame {index:1,duration_ticks:if interpolate {4} else {1}}],2,interpolate,0).unwrap(),
                                sheets:vec![SpriteMipSheet {width:2*sprite_size as u32,
                                    height:sprite_size as u32,rgba}] }
                        }).collect(),
                    }).unwrap();
                }
                animation_tick += 1;
                assert!(world.advance_atlas_animation(&mut gal,AtlasAnimationTickEvent {
                    texture_id:WORLD_MATERIAL_TEXTURE_STONE,generation:1,tick:animation_tick,
                    visible:(1..=9).collect(),animate_only_visible:true,
                }).unwrap());
                gal.retire_through_for_test(gal.latest_submission_id()).unwrap();
                assert_eq!(world.accepted_gui_atlas_incarnation(WORLD_MATERIAL_TEXTURE_STONE).unwrap(),
                    incarnation,"animation updates the same owned atlas, not a replacement binding");
            }
            // A shared texture has fewer bindings than nine different ones;
            // require constant residency across its own rebuild/reuse sequence.
            if generation == 6 && frontend.generation != 6 { previous_live = None; }
            let frame_cell = (16 * frame_scale) as usize;
            let screen_requests = requests.iter().enumerate().map(|(index,(_,local))| {
                let mut screen = local.clone();
                screen.item_raster_scale = frame_scale;
                screen.x0=index as f32*frame_cell as f32; screen.x1=screen.x0+frame_cell as f32; screen.x3=screen.x0;
                screen.y0=100.0; screen.y1=100.0; screen.y3=100.0+frame_cell as f32;
                screen.gui_width=512; screen.gui_height=512; screen.projection_extent=[512.0;2];
                if interpolate && generation == 22 {
                    // Isolate interpolation from the separately Frozen-validated
                    // lightmap quantization: unity is an explicit material input.
                    screen.material = GuiAffineMaterial::FlatItem(GuiFlatItemLighting {
                        lightmap_generation: 1, rgb: [1.0;3] });
                }
                if tinted {
                    screen.asset_id = 1;
                    screen.color_argb = ITEM_TINTS[(index + generation as usize) % 9];
                }
                if (10..14).contains(&generation) {
                    let [left,top,right,bottom] = ITEM_RECTS[(index + generation as usize) % 9].map(|v| v as f32);
                    screen.item_raster_geometry = super::super::gui_item_raster::GuiItemRasterGeometry {
                        corners:[left,top,right,top,left,bottom] };
                }
                if (14..18).contains(&generation) {
                    let [u0,v0,u1,v1] = ITEM_RECTS[(index + generation as usize) % 9].map(|v| v as f32/16.0);
                    screen.u0=u0; screen.v0=v0; screen.u1=u1; screen.v1=v1;
                }
                if (18..22).contains(&generation) {
                    screen.item_raster_geometry.corners = ITEM_ORIENTATIONS[(index + generation as usize)%8].map(|v| v as f32);
                }
                screen
            }).collect();
            let previous_generation = frontend.generation;
            let creates_before = gal.metrics().resource_creates;
            let (frame_ops,stats) = frontend.append_frame_ops_with_owned_atlases_to_target(
                &mut gal,Some(&mut world),generation,output_target,output_view,Some(output_pass),None,None,false,
                vec![],screen_requests,vec![],vec![]).unwrap();
            assert_eq!(frontend.generation, generation);
            if generation > 1 {
                if previous_generation == generation {
                    assert_eq!(gal.metrics().resource_creates, creates_before,
                        "the frame after an asset rebuild must reuse its GPU resources");
                } else {
                    assert!(gal.metrics().resource_creates > creates_before,
                        "the reload test must actually rebuild GPU resources");
                }
            }
            assert_eq!(stats.affine_quad_count,9);
            assert_eq!(stats.owned_intermediate_targets.len(),1);
            let mut ops=vec![CommandOp::Barrier(texture_barrier(output,TextureUsageState::TransferSrc,
                TextureUsageState::ColorAttachment)),CommandOp::BeginPass {pass:output_pass,target:output_target,
                colors:vec![PassAttachment {view:output_view,load_op:AttachmentLoadOp::Clear,store_op:AttachmentStoreOp::Store,
                    clear_color:Some(crate::render::vulkanic::commands::ClearColor {r:0.0,g:0.0,b:0.0,a:0.0})}],depth_stencil:None},
                CommandOp::EndPass];
            ops.extend(frame_ops);
            ops.extend([CommandOp::Barrier(texture_barrier(output,TextureUsageState::ColorAttachment,TextureUsageState::TransferSrc)),
                CommandOp::Barrier(buffer_barrier(readback,TextureUsageState::ShaderRead,TextureUsageState::TransferDst)),
                CommandOp::CopyTextureToBuffer(BufferImageCopyRegion {buffer:readback,buffer_offset:0,bytes_per_row:512*4,
                    rows_per_image:512,texture:output,texture_mip:0,texture_layer:0,texture_origin:TextureOrigin3d{x:0,y:0,z:0},extent}),
                CommandOp::Barrier(buffer_barrier(readback,TextureUsageState::TransferDst,TextureUsageState::ShaderRead)),
                CommandOp::HostReadBuffer {buffer:readback,offset:0,size:512*512*4}]);
            let list=gal.create_command_list(CommandListDesc {label:"item-raster.whole-gui.commands".into(),operations:ops}).unwrap();
            let token=gal.submit(SubmissionBatch {label:"item-raster.whole-gui.submit".into(),command_lists:vec![list]}).unwrap();
            gal.retire_through_for_test(token.submission).unwrap();
            let reads=gal.completed_host_reads();
            let actual=&reads.iter().rev().find(|read| read.buffer==readback).unwrap().bytes;
            if !tinted && generation != 22 && frame_scale == scale && sprite_size == 32 {
                assert_eq!(actual.len(),output_bytes.len());
                assert!(actual.iter().zip(output_bytes).all(|(a,b)| a == b),
                    "returning to the original GUI scale must exactly recover the independently tested two-stage raster result");
            } else {
                // Native sprites at every scale, and 32px sprites at scales
                // one/two, have exact integral texel blocks/strides. Check all
                // pixels, including vacated pixels, without a readback oracle.
                for y in 0..512 {
                    for x in 0..512 {
                        let expected = if (100..100+frame_cell).contains(&y) && x < 9*frame_cell {
                            let [left,top,right,bottom] = if (10..14).contains(&generation) {
                                ITEM_RECTS[(x/frame_cell + generation as usize) % 9]
                            } else { [0,0,16,16] }.map(|v| v * frame_scale as usize);
                            let local_x = x%frame_cell;
                            let local_y = y-100;
                            let [source_left,source_top,source_right,source_bottom] = if (14..18).contains(&generation) {
                                ITEM_RECTS[(x/frame_cell + generation as usize) % 9]
                            } else {[0,0,16,16]}.map(|v| v*sprite_size/16);
                            let source_width=source_right-source_left;
                            let source_height=source_bottom-source_top;
                            let mut column = source_left + ((local_x.saturating_sub(left)) * source_width + source_width/2) / (right-left);
                            let mut row = source_top + ((local_y.saturating_sub(top)) * source_height + source_height/2) / (bottom-top);
                            if (18..22).contains(&generation) {
                                // Exact integer inverse of the authored signed
                                // permutation, evaluated at pixel centers.
                                let [x0,y0,x1,y1,x3,y3] = ITEM_ORIENTATIONS[(x/frame_cell + generation as usize)%8];
                                let (ux,uy,vx,vy) = ((x1-x0)/16,(y1-y0)/16,(x3-x0)/16,(y3-y0)/16);
                                let (px,py) = (2*local_x as i64+1-2*x0*frame_scale as i64,
                                    2*local_y as i64+1-2*y0*frame_scale as i64);
                                let denominator = 2*frame_cell as i64*(ux*vy-uy*vx);
                                column = ((px*vy-py*vx)*sprite_size as i64/denominator) as usize;
                                row = ((py*ux-px*uy)*sprite_size as i64/denominator) as usize;
                            }
                            if generation == 22 {
                                // Independent authored black/white four-tick sequence,
                                // including both frame boundaries and the wrap to black.
                                // Do not use the production interpolation helper as oracle.
                                let value = if interpolate {
                                    [64,128,192,255,191,127,63,0][animation_tick as usize-1]
                                } else if animation_tick % 2 == 1 {252} else {0};
                                [value,value,value,255]
                            } else if !(left..right).contains(&local_x) || !(top..bottom).contains(&local_y) {
                                [0;4]
                            } else if tinted {
                                tinted_item_pixel(column,row,sprite_size,transparency && x/frame_cell == 6,
                                    transparency,ITEM_TINTS[(x/frame_cell + generation as usize) % 9])
                            } else if transparency && alpha_range.contains(&row) && alpha_range.contains(&column) {
                                if column < alpha_split { [0,0,0,0] }
                                else if x/frame_cell == 6 { [26,26,26,26] }
                                else { [3,3,3,26] }
                            } else if row == 0 { [0,0,0,255] }
                            else if column == 0 { [252,252,0,255] }
                            else if row == sprite_size-1 || column == sprite_size-1 { [17,105,48,255] }
                            else { [35,209,97,255] }
                        } else { [0,0,0,0] };
                        assert_eq!(&actual[(y*512+x)*4..(y*512+x+1)*4], expected,
                            "GUI scale transition to {frame_scale}, generation {generation}, animation tick {animation_tick} at {x},{y}");
                    }
                }
            }
            let live=gal.metrics().resource_creates-gal.metrics().resource_destroys;
            if let Some(previous)=previous_live {assert_eq!(live,previous,"repeated frames, GUI-scale changes and asset rebuilds must preserve bounded raster residency");}
            previous_live=Some(live);
        }
        let slots_before=frontend.item_raster_slots.clone();
        assert_ne!(slots_before,Default::default());
        frontend.destroy_render_resources(&mut gal);
        assert_eq!(frontend.item_raster_slots,slots_before,
            "a GUI asset-generation rebuild must not erase semantic model slot history");
    }
    composite.destroy(&mut gal);
    for handle in [output_pass,output_target,output_view,output] { gal.destroy(handle).unwrap(); }
    frontend.reset(&mut gal).unwrap();
    assert_eq!(frontend.item_raster_slots,Default::default(),"full context reset retires semantic history too");
    world.reset(&mut gal);
    raster.destroy(&mut gal).unwrap();
    gal.destroy(readback).unwrap();
    gal.retire_through(gal.latest_submission_id()).unwrap();
    assert_eq!(gal.metrics().resource_creates, gal.metrics().resource_destroys);
}
