"""Predeclared sky-only witness around the six-tick vibration snapshot."""
from PIL import ImageChops, ImageStat
import math
BOX = (700,210,900,390)

def projected_fixture_corners(steps=6, elevation=0):
    """Independent canonical camera projection of either fixed simulation pose."""
    if steps not in (0,6): raise ValueError("unsupported simulation snapshot")
    if elevation not in (0,1): raise ValueError("unsupported fixture elevation")
    from vibration_reference import expected_position
    position = expected_position(steps, elevation)
    eye = (150.5,101.62,530.5)
    yaw,pitch = math.radians(105),math.radians(10)
    forward = (-math.sin(yaw)*math.cos(pitch),-math.sin(pitch),math.cos(yaw)*math.cos(pitch))
    right = (-math.cos(yaw),0,-math.sin(yaw))
    up = (-math.sin(yaw)*math.sin(pitch),math.cos(pitch),math.cos(yaw)*math.sin(pitch))
    def dot(a,b): return sum(x*y for x,y in zip(a,b))
    def project(point):
        offset = tuple(a-b for a,b in zip(point,eye))
        scale = 360/math.tan(math.radians(35))/dot(offset,forward)
        return 640+scale*dot(offset,right),360-scale*dot(offset,up)
    def ry(v,a):
        x,y,z=v
        return math.cos(a)*x+math.sin(a)*z,y,-math.sin(a)*x+math.cos(a)*z
    def rx(v,a):
        x,y,z=v
        return x,math.cos(a)*y-math.sin(a)*z,math.sin(a)*y+math.cos(a)*z
    delta = tuple(a-b for a,b in zip(position,(149.5,102.5+elevation,526.5)))
    h = math.atan2(delta[0],delta[2])
    i = math.atan2(delta[1],math.hypot(delta[0],delta[2]))+math.pi/2
    g = math.sin((13+steps-2*math.pi)*0.05)*2
    corners=[]
    for yrot,xrot in ((h,-i),(h-math.pi,i)):
        for x,y in ((1,-1),(1,1),(-1,1),(-1,-1)):
            offset = ry(rx(ry((x*0.3,y*0.3,0),g),xrot),yrot)
            corners.append(project(tuple(a+b for a,b in zip(position,offset))))
    return project(position),corners

def local(a,b):
    if a.size != (1280,720) or b.size != a.size: return {"passed":False,"error":"extent"}
    regions=[]
    for y in range(210,390,30):
        for x in range(700,900,25):
            box=(x,y,x+25,y+30)
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
    if any(p.size != (1280,720) for p in (hidden,baseline,current)): return {"passed":False,"error":"extent"}
    changes=ImageChops.difference(hidden.crop(BOX),baseline.crop(BOX))
    errors=ImageChops.difference(baseline.crop(BOX),current.crop(BOX))
    pixels=[e for c,e in zip(changes.getdata(),errors.getdata()) if max(c)>3]
    mean=[sum(p[c] for p in pixels)/len(pixels) for c in range(3)] if pixels else []
    return {"passed":len(pixels)>=32 and max(mean)<=6,"pixels":len(pixels),"mean_rgb_abs":mean}
