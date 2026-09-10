"""Independent CPU witness for the authored standard GUI foil sampling fixture.

Inputs are observed source vertices, not candidate image pixels or native output.
This module has no renderer/backend handles and does not relax image thresholds.
"""
import math
import struct


def f32(value):
    try:
        result = struct.unpack("f", struct.pack("f", value))[0]
    except (OverflowError, struct.error) as error:
        raise ValueError("foil reference float overflow") from error
    if not math.isfinite(result):
        raise ValueError("foil reference non-finite float")
    return result


def animation_offset(clock_millis, speed):
    """Observe Java's double multiply, saturating long cast and f32 division.

    The input is the actual render clock, never world ticks or capture time.
    Multiplication must precede remainder reduction (including near LONG_MAX).
    """
    if (type(clock_millis) is not int or not 0 <= clock_millis <= 2**63-1
            or isinstance(speed, bool) or not isinstance(speed, (int,float))
            or not math.isfinite(speed) or not 0 <= speed <= 1):
        raise ValueError("invalid foil animation inputs")
    ticks = min(2**63-1, int(float(clock_millis)*float(speed)*8.0))
    return scaled_offset(ticks)


def scaled_offset(ticks):
    if type(ticks) is not int or not 0 <= ticks <= 2**63-1:
        raise ValueError("invalid observed scaled foil ticks")
    return (-f32(f32(ticks % 110000)/110000), f32(f32(ticks % 30000)/30000))


def hand_timing_evidence(receipt, phase):
    if not isinstance(receipt,dict) or type(phase) is not int or not 0 <= phase < 330000:
        raise ValueError("invalid hand foil phase receipt")
    hand = receipt.get("hand")
    if (not isinstance(hand,dict) or hand.get("enabled") is not True or hand.get("complete") is not True
            or type(hand.get("frameSequence")) is not int or hand["frameSequence"] <= 0):
        raise ValueError("missing capture-local hand foil timing")
    ticks = hand.get("scaledTicks")
    if (not isinstance(ticks,list) or len(ticks)!=1 or type(ticks[0]) is not int
            or not 0 <= ticks[0] <= 2**63-1 or (ticks[0]-phase)%330000 > 512):
        raise ValueError("hand foil phase is absent, ambiguous, or outside requested window")
    return ticks[0]


def timing_evidence(receipt, scale, viewport):
    from graphics_harness import flat_item_witness_layout
    if (not isinstance(receipt,dict) or receipt.get("enabled") is not True
            or receipt.get("complete") is not True or type(receipt.get("frameSequence")) is not int
            or receipt["frameSequence"] <= 0):
        raise ValueError("missing or incomplete capture-local foil timing")
    samples=receipt.get("samples")
    if not isinstance(samples,list) or len(samples)!=8:
        raise ValueError("foil timing must contain every enchanted slot exactly once")
    boxes,_=flat_item_witness_layout(scale,viewport)
    expected=[((box[0]-1)//scale,box[1]//scale) for box in boxes[1:]]
    found={}
    for sample in samples:
        position=(sample.get("x"),sample.get("y"))
        if any(type(v) is not int for v in position) or position not in expected or position in found:
            raise ValueError("ambiguous foil timing slot")
        if "scaledTicks" in sample:
            if any(k in sample for k in ("clockMillis","speed","strength")):
                raise ValueError("mixed timing representations")
            ticks=sample["scaledTicks"]
        else:
            clock=sample.get("clockMillis")
            if type(clock) is not int or not 0 <= clock <= 2**63-1 or sample.get("speed")!=.5 or sample.get("strength")!=.5:
                raise ValueError("moving foil semantic clock or settings invalid")
            ticks=min(2**63-1,int(float(clock)*.5*8))
        scaled_offset(ticks)
        if ticks==0: raise ValueError("moving foil requires a running observed clock")
        found[position]=ticks
    return [found[position] for position in expected]


def temporal_change_evidence(before, after):
    """Require the predicted change, not merely any image difference.

    Each independent per-frame probe is already bounded by two channel levels,
    so the difference of those observations may deviate by at most four.
    Every enchanted slot must have a predicted and observed visible change on
    BOTH backends. Reusing a stale frame or an unchanged phase fails.
    """
    if any(row.get("variant")!="foil-moving" or row.get("passed") is not True
           or len(row.get("items",[]))!=9 for row in (before,after)):
        raise ValueError("temporal foil evidence requires two complete moving-phase rows")
    changed=[]
    for index,(old,new) in enumerate(zip(before["items"],after["items"])):
        if old["item"]!=new["item"]:
            raise ValueError("temporal foil item identity differs")
        if index==0: continue
        probes_before=old.get("orientation_samples",[])
        probes_after=new.get("orientation_samples",[])
        if len(probes_before)!=4 or len(probes_after)!=4:
            raise ValueError("temporal foil requires all quadrant probes")
        for backend in range(2):
            visible=False
            accurate=True
            for a,b in zip(probes_before,probes_after):
                expected=[y-x for x,y in zip(a["expected"][backend],b["expected"][backend])]
                observed=[y-x for x,y in zip(a["observed"][backend],b["observed"][backend])]
                accurate &= all(abs(x-y)<=4 for x,y in zip(expected,observed))
                visible |= any(abs(x)>4 and abs(y)>2 and x*y>0 for x,y in zip(expected,observed))
            changed.append({"item":old["item"],"backend":backend,"predicted_change_observed":visible,
                            "delta_accurate":accurate,"passed":visible and accurate})
    return {"passed":bool(changed) and all(row["passed"] for row in changed),"items":changed}


def pattern_pixel(x, y):
    if not (0 <= x < 16 and 0 <= y < 16):
        raise ValueError("pattern coordinate outside authored texture")
    return (64 + (13*x + 7*y) % 128, 48 + (5*x + 17*y) % 144,
            80 + (11*x + 3*y) % 112, 255)


def standard_uv(uv, clock_millis=0, speed=0.0, scaled_ticks=None):
    """Frozen's translate/rotateZ(float(PI/18))/scale(8), including time."""
    if len(uv) != 2 or not all(math.isfinite(v) for v in uv):
        raise ValueError("invalid source UV")
    angle = f32(math.pi / 18)
    sine = f32(math.sin(angle))
    cosine = f32(math.sqrt(f32(1 - f32(sine*sine))))
    s, c = f32(sine*8), f32(cosine*8)
    dx, dy = animation_offset(clock_millis, speed) if scaled_ticks is None else scaled_offset(scaled_ticks)
    return (f32(f32(f32(c*uv[0]) - f32(s*uv[1])) + dx),
            f32(f32(f32(s*uv[0]) + f32(c*uv[1])) + dy))


def sample_pattern(uv, *, blur=True, clamp=False):
    """Independent normalized sampler for the authored 16x16 foil texture."""
    if len(uv) != 2 or not all(math.isfinite(v) for v in uv):
        raise ValueError("invalid sampling UV")
    if type(blur) is not bool or type(clamp) is not bool:
        raise ValueError("sampling metadata must contain booleans")
    def address(index):
        return min(15, max(0, index)) if clamp else index % 16
    # Bound coordinates before conversion, including large finite inputs.
    coords = [min(1.0, max(0.0, v)) if clamp else v % 1 for v in uv]
    if not blur:
        return pattern_pixel(*(address(math.floor(v*16)) for v in coords))[:3]
    x, y = (v*16 - 0.5 for v in coords)
    ix, iy = math.floor(x), math.floor(y)
    fx, fy = x-ix, y-iy
    pixels = [pattern_pixel(address(a), address(b)) for a,b in
              ((ix,iy),(ix+1,iy),(ix,iy+1),(ix+1,iy+1))]
    return tuple((pixels[0][c]*(1-fx)+pixels[1][c]*fx)*(1-fy)
                 +(pixels[2][c]*(1-fx)+pixels[3][c]*fx)*fy for c in range(3))


def source_at_pixel(sample, x, y, scale, clock_millis=0, speed=0.0, scaled_ticks=None):
    """Interpolate the actual source quad at a full-cell GUI pixel center."""
    positions, uvs = sample.get("positions"), sample.get("atlasUvs")
    if (not isinstance(positions, list) or len(positions) != 12
            or not isinstance(uvs, list) or len(uvs) != 8
            or not all(isinstance(v, (int,float)) and math.isfinite(v) for v in positions+uvs)
            or scale not in (1,2,3)):
        raise ValueError("invalid observed source quad")
    if len(set(positions[2::3])) != 1:
        raise ValueError("non-planar foil fixture source")
    px, py = (x+0.5/scale)/16, 1-(y+0.5/scale)/16
    for triangle in ((0,1,2),(2,3,0)):
        a,b,c = [(positions[i*3],positions[i*3+1]) for i in triangle]
        determinant = (b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
        if abs(determinant) < 1e-12:
            raise ValueError("degenerate foil fixture triangle")
        wa = ((b[1]-c[1])*(px-c[0])+(c[0]-b[0])*(py-c[1]))/determinant
        wb = ((c[1]-a[1])*(px-c[0])+(a[0]-c[0])*(py-c[1]))/determinant
        weights = (wa,wb,1-wa-wb)
        if min(weights) >= -1e-9:
            transformed = [standard_uv(uvs[i*2:i*2+2], clock_millis, speed, scaled_ticks) for i in triangle]
            return tuple(sum(weight*uv[axis] for weight,uv in zip(weights,transformed)) for axis in (0,1))
    raise ValueError("foil probe outside observed source geometry")


def expected_pixel(base_rgb, sample, x, y, scale, clock_millis=0, speed=0.0, strength=0.5, scaled_ticks=None, *, blur=True, clamp=False):
    if (isinstance(strength, bool) or not isinstance(strength, (int,float))
            or not math.isfinite(strength) or not 0 <= strength <= 1):
        raise ValueError("invalid foil strength")
    strength = f32(strength)
    lighted = [round(v*252/255) for v in base_rgb]
    sampled = sample_pattern(source_at_pixel(sample,x,y,scale,clock_millis,speed,scaled_ticks), blur=blur, clamp=clamp)
    return [min(255, round(base+(channel*strength)**2/255)) for base,channel in zip(lighted,sampled)]


def sampler_filter_discrimination(source, scale):
    """Require predetermined GUI probes to separate the two filter oracles.

    A difference greater than twice the unchanged pixel tolerance (2) means
    the same observed pixel cannot pass both filters' expected colors.
    """
    from capture_runner import FLAT_ITEM_UV_COLORS
    rows=[]
    for (x,y),color in zip(((4,4),(12,4),(4,12),(12,12)), FLAT_ITEM_UV_COLORS):
        nearest=expected_pixel(color,source,x,y,scale,blur=False)
        linear=expected_pixel(color,source,x,y,scale,blur=True)
        difference=max(abs(a-b) for a,b in zip(nearest,linear))
        rows.append(dict(position=[x,y],nearest=nearest,linear=linear,max_difference=difference))
    return dict(passed=any(row["max_difference"]>4 for row in rows),probes=rows)


def source_evidence(receipt):
    if not isinstance(receipt,dict) or receipt.get("enabled") is not True or receipt.get("complete") is not True:
        raise ValueError("foil source observations are absent or ambiguous")
    sources = receipt.get("sources")
    if not isinstance(sources,list) or not 1 <= len(sources) <= 64:
        raise ValueError("invalid foil source observation count")
    result = {}
    for sample in sources:
        name = sample.get("sprite") if isinstance(sample,dict) else None
        if not isinstance(name,str) or name in result:
            raise ValueError("invalid or duplicate foil source identity")
        source_at_pixel(sample,4,4,2)
        result[name] = sample
    return result
