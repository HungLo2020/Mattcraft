"""Observed standard block-item foil, with per-icon positive/control witnesses."""
import json
import math
from pathlib import Path
from PIL import Image, ImageChops, ImageStat
from gui_special_foil_reference import image_pair, observed_timing


def sources(receipt):
    if not isinstance(receipt,dict) or receipt.get("enabled") is not True or receipt.get("complete") is not True:
        raise ValueError("block foil sources missing or ambiguous")
    result={}
    rows=receipt.get("sources",[])
    if not isinstance(rows,list) or not 9 <= len(rows) <= 64:
        raise ValueError("block foil requires the full fixture source set")
    for row in rows:
        if not isinstance(row,dict): raise ValueError("invalid block foil source record")
        name=row.get("sprite")
        positions,uv=row.get("positions",[]),row.get("atlasUvs",[])
        if not isinstance(name,str) or name in result or len(positions)!=12 or len(uv)!=8:
            raise ValueError("invalid block foil source shape")
        if any(type(v) not in (int,float) or not math.isfinite(v) for v in positions+uv):
            raise ValueError("nonfinite block foil source")
        a=[positions[i+3]-positions[i] for i in range(3)]
        b=[positions[i+6]-positions[i] for i in range(3)]
        cross=[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
        if sum(v*v for v in cross)<=1e-12 or any(max(uv[axis::2])<=min(uv[axis::2]) for axis in (0,1)):
            raise ValueError("degenerate block foil source")
        result[name]=row
    required={"stone","grass_block_side","redstone_ore","oak_leaves","oak_planks",
              "oak_trapdoor","white_wool","crafting_table_side","oak_log"}
    if not {"minecraft:block/"+name for name in required}.issubset(result):
        raise ValueError("block foil fixture source identities incomplete")
    return result


def foil_residual(frozen,current,before_frozen,before_current):
    """Signed enchanted-minus-unenchanted image error and base differences.

    Diagnostic only: never replaces or relaxes whole-icon acceptance. Work in
    signed integers because ImageChops.subtract clips negative image deltas.
    The difference can include changed base-normal selection, not just glint.
    """
    if len({im.size for im in (frozen,current,before_frozen,before_current)})!=1:
        raise ValueError("foil residual image extents differ")
    pixels=[list(im.convert("RGB").get_flattened_data())
            for im in (frozen,current,before_frozen,before_current)]
    if not pixels[0]: raise ValueError("empty foil residual image")
    base_total=[0,0,0]; effect_total=[0,0,0]
    base_max=0; effect_max=0; effect_pixels_over_eight=0
    for frozen_pixel,current_pixel,old_frozen,old_current in zip(*pixels):
        base=[abs(old_current[c]-old_frozen[c]) for c in range(3)]
        effect=[abs((current_pixel[c]-old_current[c])-(frozen_pixel[c]-old_frozen[c])) for c in range(3)]
        for c in range(3):
            base_total[c]+=base[c]; effect_total[c]+=effect[c]
        base_max=max(base_max,*base); effect_max=max(effect_max,*effect)
        effect_pixels_over_eight+=max(effect)>8
    count=len(pixels[0])
    return dict(base_mean_channel_error=[v/count for v in base_total],
                base_max_channel_error=base_max,
                foil_effect_mean_channel_error=[v/count for v in effect_total],
                foil_effect_max_channel_error=effect_max,
                foil_effect_pixels_over_eight=effect_pixels_over_eight,pixel_count=count)


def images(frozen,current,before_frozen,before_current,scale):
    if len({im.size for im in (frozen,current,before_frozen,before_current)})!=1:
        raise ValueError("block foil image extents differ")
    result=image_pair(frozen,current,scale)
    before=image_pair(before_frozen,before_current,scale)
    result["schema"]="gui-standard-block-foil-v1"
    result["reference"]=before
    for row in result["probes"]:
        changes=[max(ImageStat.Stat(ImageChops.difference(a.convert("RGB").crop(row["box"]),
            b.convert("RGB").crop(row["box"]))).mean) for a,b in ((frozen,before_frozen),(current,before_current))]
        row["changes"]=changes
        row["residual_diagnostic"]=foil_residual(*(im.crop(row["box"])
            for im in (frozen,current,before_frozen,before_current)))
        row["foil_or_control_verified"]=all(v<=2 for v in changes) if row["slot"]==1 else all(v>4 for v in changes)
        row["passed"] &= row["foil_or_control_verified"]
    result["passed"]=before["passed"] and all(row["passed"] for row in result["probes"])
    return result


def report(visual,scenario,reference):
    if scenario not in ("block-item-foil", "block-item-foil-moving"):
        return dict(requested=False,passed=True,pairs=[])
    phase=10000 if scenario=="block-item-foil-moving" else None
    import graphics_harness as h
    rows=[]
    try:
        if reference is None: raise ValueError("block foil requires accepted unfoiled reference")
        prior=h.read_json(Path(reference))
        if prior.get("success") is not True or prior.get("gui_block_lighting_parity",{}).get("passed") is not True:
            raise ValueError("block foil reference must have accepted block lighting")
        old=prior["cross_repository_visual_parity"]["pairs"]
        pairs=visual.get("pairs",[])
        if len(old)!=1 or len(pairs)!=1: raise ValueError("block foil requires one matching camera pair")
        metas=[]; observed=[]
        for pair in (pairs[0],old[0]):
            for key in ("baseline_artifact","current_artifact"):
                artifact=Path(pair[key]); doc=h.deterministic_capture_document(artifact) or {}
                current=pair is pairs[0]
                if doc.get("hotbarItemFixture")!="standard-3d-logs" or doc.get("selectedHotbarSlot")!=1 or doc.get("guiItemFoilCount")!=(8 if current else 0):
                    raise ValueError("actual block foil inventory differs")
                if doc.get("guiItemPlacement") is not None: raise ValueError("ordinary foil fixture cannot include transformed placement")
                if current:
                    if doc.get("guiItemGlintSpeed")!=(0.5 if phase is not None else 0) or doc.get("guiItemGlintStrength")!=0.5:
                        raise ValueError("actual block foil settings differ")
                    observed.append(sources(doc.get("guiItemFoilSources")))
                reload=doc.get("worldResourceReload",{})
                expected=["vanilla","file/mattmc-block-item-foil"] if current else ["vanilla"]
                if reload.get("complete") is not True or reload.get("selectedAtCapture")!=expected:
                    raise ValueError("actual block foil reload/pack differs")
                meta=h.latest_capture_meta_path(artifact.parent/"capture")
                if meta is None: raise ValueError("missing block foil launch receipt")
                metas.append(h.read_key_values(meta))
        if observed[0]!=observed[1]: raise ValueError("block foil source geometry/UVs differ")
        for key in ("forced_option_guiScale","parity_fixture_id","parity_fixture_source_save_hash","parity_camera_x",
                    "parity_camera_y","parity_camera_z","parity_camera_yaw","parity_camera_pitch"):
            if not metas[0].get(key) or len({meta.get(key) for meta in metas})!=1:
                raise ValueError("block foil reference differs: "+key)
        scale=int(metas[0]["forced_option_guiScale"])
        with Image.open(pairs[0]["baseline_image"]) as a, Image.open(pairs[0]["current_image"]) as b, \
             Image.open(old[0]["baseline_image"]) as c, Image.open(old[0]["current_image"]) as d:
            timings=[observed_timing(h.read_json(Path(str(pairs[0][key])+".foil-timing.json")),scale,a.size,phase)
                     for key in ("baseline_image","current_image")]
            row=images(a,b,c,d,scale)
            row["observed_scaled_ticks"]=timings
            row["requested_phase"]=phase
            row["sources_and_settings_verified"]=True
            rows.append(row)
    except (KeyError,OSError,TypeError,ValueError) as error:
        rows.append(dict(passed=False,reason=str(error)))
    return dict(requested=True,passed=bool(rows) and all(row["passed"] for row in rows),pairs=rows)
