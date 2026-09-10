"""Strict enlarged GUI item check, supplemental to the shared world parity gates."""
import json
import math
from pathlib import Path
from PIL import Image, ImageChops, ImageStat


def images(frozen, current, old_frozen, old_current, scale):
    if scale not in (2, 3) or len({im.size for im in (frozen,current,old_frozen,old_current)}) != 1:
        raise ValueError("equivalent GUI scale and image extents required")
    width,height = frozen.size
    # Includes the whole enlarged middle icon and padding, not hand-picked faces.
    box = (width//2-19*scale,height-35*scale,width//2+19*scale,height-2*scale)
    if box[0] < 0 or box[1] < 0 or box[2] > width:
        raise ValueError("capture too small")
    crops = [im.convert("RGB").crop(box) for im in (frozen,current,old_frozen,old_current)]
    def error(a,b):
        return max(ImageStat.Stat(ImageChops.difference(a,b)).mean)
    pair_error = error(crops[0],crops[1])
    changes = [error(crops[i],crops[i+2]) for i in (0,1)]
    visible = all(max(ImageStat.Stat(im).stddev) > 8 for im in crops)
    return dict(box=box,max_mean_channel_error=pair_error,changes=changes,visible=visible,
                passed=visible and pair_error <= 2.0 and all(value > 4.0 for value in changes))


def placement_receipt(doc):
    value=doc.get("guiItemPlacement")
    if not isinstance(value,dict) or value.get("schema") != "gui-item-placement-v1" or value.get("item") != "minecraft:oak_slab" or value.get("submitted") is not True:
        raise ValueError("missing actual GUI item placement receipt")
    x,y=value.get("x"),value.get("y")
    if type(x) is not int or type(y) is not int or value.get("originX") != x-512 or value.get("clip") != [x,y-12,14,17]:
        raise ValueError("unexpected GUI item clip rectangle")
    pose=value.get("pose",[])
    if len(pose)!=6 or any(type(v) not in (int,float) or not math.isfinite(v) for v in pose):
        raise ValueError("invalid GUI item pose")
    c,s=math.cos(0.2),math.sin(0.2)
    expected=[c*1.15,s*1.15,-s*0.9,c*0.9]
    expected += [x+8-(x-512+8)*expected[0]-(y+8)*expected[2],
                 y+1-(x-512+8)*expected[1]-(y+8)*expected[3]]
    if any(abs(a-b)>0.0001 for a,b in zip(pose,expected)):
        raise ValueError("GUI item transform differs from shared fixture")
    return value


def report(visual, scenario, reference, placement=False):
    if scenario not in ("block-item-expanded", "block-item-oversized"):
        return dict(requested=False,passed=True,pairs=[])
    from graphics_harness import latest_capture_meta_path, read_key_values, deterministic_capture_document
    rows=[]
    try:
        if reference is None:
            raise ValueError("expanded item requires accepted no-pack reference")
        previous=json.loads(Path(reference).read_text())
        if previous.get("success") is not True or previous.get("gui_block_lighting_parity",{}).get("passed") is not True:
            raise ValueError("reference must have accepted GUI block lighting")
        old=previous["cross_repository_visual_parity"]["pairs"]
        pairs=visual.get("pairs",[])
        if len(pairs) != 1 or len(old) != 1:
            raise ValueError("one equivalent camera pair required")
        metas=[]
        placements=[]
        for pair in (pairs[0],old[0]):
            for key in ("baseline_artifact","current_artifact"):
                doc=deterministic_capture_document(Path(pair[key])) or {}
                if doc.get("hotbarItemFixture") != "standard-3d-logs":
                    raise ValueError("actual standard-3d-logs fixture required")
                if pair is pairs[0] and placement:
                    placements.append(placement_receipt(doc))
                elif doc.get("guiItemPlacement") is not None:
                    raise ValueError("unexpected placement fixture in baseline")
                reload=doc.get("worldResourceReload",{})
                expected=["vanilla","file/mattmc-"+scenario] if pair is pairs[0] else ["vanilla"]
                if reload.get("complete") is not True or reload.get("selectedAtCapture") != expected:
                    raise ValueError("actual selected packs after reload differ")
                path=latest_capture_meta_path(Path(pair[key]).parent/"capture")
                if path is None: raise ValueError("missing actual launch receipt")
                metas.append(read_key_values(path))
        scales=[int(meta.get("forced_option_guiScale","0")) for meta in metas]
        if placement and (len(placements)!=2 or placements[0]!=placements[1]):
            raise ValueError("actual GUI item placements differ between backends")
        if len(set(scales)) != 1: raise ValueError("reference GUI scales differ")
        for key in ("parity_fixture_id","parity_fixture_source_save_hash","parity_camera_x",
                    "parity_camera_y","parity_camera_z","parity_camera_yaw","parity_camera_pitch"):
            if not metas[0].get(key) or len({meta.get(key) for meta in metas}) != 1:
                raise ValueError("reference fixture/camera differs: "+key)
        for meta in metas[:2]:
            if json.loads(meta.get("gui_resource_pack_selected","[]")) != ["file/mattmc-"+scenario]:
                raise ValueError("expanded pack must actually be selected")
        if any(json.loads(meta.get("gui_resource_pack_selected","[]")) not in ([],["vanilla"]) for meta in metas[2:]):
            raise ValueError("reference must use vanilla resources")
        with Image.open(pairs[0]["baseline_image"]) as a, Image.open(pairs[0]["current_image"]) as b, \
             Image.open(old[0]["baseline_image"]) as c, Image.open(old[0]["current_image"]) as d:
            rows.append(images(a,b,c,d,scales[0]))
            if placement: rows[-1]["placements"]=placements
    except (KeyError,OSError,TypeError,ValueError) as error:
        rows.append(dict(passed=False,reason=str(error)))
    return dict(requested=True,passed=bool(rows) and all(row["passed"] for row in rows),pairs=rows)
