package net.minecraft.client.particle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.util.function.Supplier;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

/** Capture-only particle inputs. Ordinary particle construction is unchanged. */
public final class GraphicsAuditTerrainParticleFixture {
    private static final ThreadLocal<Boolean> CONSTRUCTING = new ThreadLocal<>();
    private static ClientLevel installedLevel;
    private static TerrainParticle particle;
    private GraphicsAuditTerrainParticleFixture() {}

    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditMagmaParticle") || cutoutRequested() || translucentRequested();
    }

    public static boolean cutoutRequested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditCutoutTerrainParticle");
    }

    public static boolean translucentRequested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditTranslucentTerrainParticle");
    }

    static boolean hidden() {
        return translucentRequested() ? Boolean.getBoolean("mattmc.dev.graphicsAuditTranslucentTerrainHidden")
            : cutoutRequested() && Boolean.getBoolean("mattmc.dev.graphicsAuditCutoutTerrainHidden");
    }

    static net.minecraft.world.level.block.state.BlockState fixtureBlock() {
        if (cutoutRequested() && translucentRequested()) throw new IllegalStateException("ambiguous terrain particle fixture");
        return (translucentRequested() ? Blocks.BLUE_STAINED_GLASS
            : cutoutRequested() ? Blocks.OAK_LEAVES : Blocks.MAGMA_BLOCK).defaultBlockState();
    }

    static <T> T construct(Supplier<T> factory) {
        if (Boolean.TRUE.equals(CONSTRUCTING.get())) throw new IllegalStateException("nested particle fixture construction");
        CONSTRUCTING.set(true);
        try { return factory.get(); } finally { CONSTRUCTING.remove(); }
    }

    static float offset(float ordinary) {
        return Boolean.TRUE.equals(CONSTRUCTING.get()) ? 1.0F : ordinary;
    }

    static void configure(TerrainParticle value) {
        if (!Boolean.TRUE.equals(CONSTRUCTING.get())) return;
        value.quadSize = 0.35F;
        value.gravity = 0;
        value.hasPhysics = false;
        value.xd = value.yd = value.zd = 0;
        value.lifetime = 20000;
        value.friction = 1;
        if (cutoutRequested() || translucentRequested()) value.alpha = hidden() ? 0 : 1;
    }

    static Vec3 position(Vec3 eye, Vec3 look) {
        if (look.lengthSqr() < 0.0001) throw new IllegalArgumentException("missing fixture look direction");
        return eye.add(look.normalize().scale(3));
    }

    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        if (installedLevel == minecraft.level && particle != null) return;
        Vec3 origin = position(minecraft.player.getEyePosition(), minecraft.player.getLookAngle());
        if (cutoutRequested() || translucentRequested()) origin = origin.add(0,1,0);
        final Vec3 particleOrigin = origin;
        particle = construct(() -> new TerrainParticle(minecraft.level, particleOrigin.x, particleOrigin.y, particleOrigin.z,
            0, 0, 0, fixtureBlock()));
        installedLevel = minecraft.level;
        minecraft.particleEngine.installGraphicsAuditParticle(particle);
    }

    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        JsonObject result = new JsonObject();
        result.addProperty("fixture", translucentRequested() ? "blue-glass-translucent-terrain-v1"
            : cutoutRequested() ? "oak-leaves-cutout-terrain-v1" : "magma-terrain-particle-v1");
        boolean present = installedLevel == minecraft.level && particle != null;
        result.addProperty("complete", present && particle.isAlive()
            && minecraft.particleEngine.containsGraphicsAuditParticle(particle));
        if (!present) return result.toString();
        result.addProperty("block", translucentRequested() ? "minecraft:blue_stained_glass"
            : cutoutRequested() ? "minecraft:oak_leaves" : "minecraft:magma_block");
        result.addProperty("sprite", particle.sprite.contents().name().toString());
        JsonArray position = new JsonArray();
        position.add(particle.x); position.add(particle.y); position.add(particle.z);
        result.add("position", position);
        JsonArray uv = new JsonArray();
        uv.add(particle.sprite.getUOffset(particle.getU0()));
        uv.add(particle.sprite.getUOffset(particle.getU1()));
        uv.add(particle.sprite.getVOffset(particle.getV0()));
        uv.add(particle.sprite.getVOffset(particle.getV1()));
        result.add("localUv", uv);
        JsonArray color = new JsonArray();
        color.add(particle.rCol); color.add(particle.gCol); color.add(particle.bCol); color.add(particle.alpha);
        result.add("color", color);
        result.addProperty("size", particle.getQuadSize(1));
        result.addProperty("light", particle.getLightColor(1));
        if (cutoutRequested() || translucentRequested()) {
            result.addProperty("hidden", hidden());
            result.addProperty("alphaTested", particle.graphicsAuditAlphaTested());
            result.addProperty("translucent", particle.getLayer().translucent());
            var source = particle.sprite.contents();
            result.addProperty("staticSource", source.animatedTexture == null);
            result.add("sourcePixels", sourcePixels(source.originalImage));
        }
        return result.toString();
    }

    static JsonObject sourcePixels(net.blaze3d.platform.NativeImage pixels) {
        var result = new JsonObject();
        int width=pixels.getWidth(), height=pixels.getHeight();
        long hash=0xcbf29ce484222325L;
        int transparent=0, opaque=0, translucent=0;
        for(int y=0;y<height;y++) for(int x=0;x<width;x++) {
            int argb=pixels.getPixel(x,y);
            for(int shift : new int[]{16,8,0,24}) hash=(hash^((argb>>>shift)&255))*0x100000001b3L;
            // The fixture's uo/vo=1 selects exactly this quarter-sprite region.
            if(x>=width/4 && x<width/2 && y>=height/4 && y<height/2) {
                int alpha=argb>>>24;
                if(alpha==0) transparent++;
                if(alpha==255) opaque++;
                if(alpha>0 && alpha<255) translucent++;
            }
        }
        result.addProperty("width",width); result.addProperty("height",height);
        result.addProperty("rgbaFnv64",String.format(java.util.Locale.ROOT,"%016x",hash));
        result.addProperty("quarterTransparentPixels",transparent);
        result.addProperty("quarterOpaquePixels",opaque);
        result.addProperty("quarterTranslucentPixels",translucent);
        return result;
    }
}
