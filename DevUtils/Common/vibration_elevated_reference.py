"""Full-geometry sky witness for the translated ordinary vibration fixture."""
from PIL import ImageChops, ImageStat

# Canonical independent projection bounds: x588.67..695.62, y135.41..230.39.
BOX = (560,120,720,240)

def local(a,b):
    if a.size != (1280,720) or b.size != a.size: return {"passed":False,"error":"extent"}
    regions=[]
    for y in range(120,240,30):
        for x in range(560,720,40):
            box=(x,y,x+40,y+30)
            mean=ImageStat.Stat(ImageChops.difference(a.crop(box),b.crop(box))).mean
            regions.append({"box":box,"mean_rgb_abs":mean,"passed":max(mean)<=6})
    return {"passed":all(r["passed"] for r in regions),"regions":regions}

def effect(a,b):
    if a.size != (1280,720) or b.size != a.size: return {"passed":False,"error":"extent"}
    delta=ImageChops.difference(a.crop(BOX),b.crop(BOX))
    mean=ImageStat.Stat(delta).mean
    changed=sum(max(p)>3 for p in delta.getdata())
    return {"passed":max(mean)>0.1 and changed>=32,"mean_rgb_change":mean,"changed_pixels":changed}

def changed_pixel_parity(hidden,baseline,current):
    from vibration_reference import full_quad_changed_pixel_parity
    return full_quad_changed_pixel_parity(hidden,baseline,current,elevation=1)
