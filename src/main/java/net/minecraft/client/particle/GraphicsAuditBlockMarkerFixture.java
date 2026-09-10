package net.minecraft.client.particle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LightBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

/** Opt-in capture fixture; never changes ordinary marker construction or rendering. */
public final class GraphicsAuditBlockMarkerFixture {
    static final String PROPERTY = "mattmc.dev.graphicsAuditNativeMarker";
    private static ClientLevel installedLevel;
    private static BlockMarker particle;
    private GraphicsAuditBlockMarkerFixture() {}

    static String scenario() { return System.getProperty(PROPERTY, "").strip(); }
    public static boolean requested() { return !scenario().isEmpty(); }
    static BlockState state(String value) {
        if ("barrier".equals(value)) return Blocks.BARRIER.defaultBlockState();
        if (value.matches("light-(?:[0-9]|1[0-5])"))
            return Blocks.LIGHT.defaultBlockState().setValue(LightBlock.LEVEL, Integer.parseInt(value.substring(6)));
        throw new IllegalArgumentException("unsupported native marker fixture: " + value);
    }
    static boolean hidden() { return Boolean.getBoolean("mattmc.dev.graphicsAuditNativeMarkerHidden"); }
    static Vec3 position(Vec3 eye, Vec3 look) {
        return GraphicsAuditTerrainParticleFixture.position(eye, look).add(0, 1, 0);
    }
    static void configure(BlockMarker value) {
        value.gravity = 0;
        value.hasPhysics = false;
        value.xd = value.yd = value.zd = 0;
        value.lifetime = 20000;
        value.friction = 1;
        value.alpha = hidden() ? 0 : 1;
    }
    static boolean retained(ParticleEngine engine, Particle value) {
        return value != null && value.isAlive() && engine.containsGraphicsAuditParticle(value);
    }
    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        // Normal resource reload clears particle groups. A detached old sprite
        // cannot stand in for the newly loaded resource in a capture fixture.
        if (installedLevel == minecraft.level && retained(minecraft.particleEngine, particle)) return;
        Vec3 p = position(minecraft.player.getEyePosition(), minecraft.player.getLookAngle());
        BlockMarker created = new BlockMarker(minecraft.level, p.x, p.y, p.z, state(scenario()));
        configure(created);
        minecraft.particleEngine.installGraphicsAuditParticle(created);
        particle = created;
        installedLevel = minecraft.level;
    }
    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        var out = new JsonObject();
        out.addProperty("fixture", "native-block-marker-v1");
        out.addProperty("scenario", scenario());
        boolean present = installedLevel == minecraft.level && particle != null;
        out.addProperty("complete", present && particle.isAlive()
            && minecraft.particleEngine.containsGraphicsAuditParticle(particle));
        if (!present) return out.toString();
        out.addProperty("hidden", hidden());
        out.addProperty("sprite", particle.sprite.contents().name().toString());
        out.addProperty("translucent", particle.getLayer().translucent());
        out.addProperty("size", particle.getQuadSize(1));
        out.addProperty("light", particle.getLightColor(1));
        var pos = new JsonArray();
        pos.add(particle.x); pos.add(particle.y); pos.add(particle.z);
        out.add("position", pos);
        var uv = new JsonArray();
        uv.add(particle.sprite.getUOffset(particle.getU0()));
        uv.add(particle.sprite.getUOffset(particle.getU1()));
        uv.add(particle.sprite.getVOffset(particle.getV0()));
        uv.add(particle.sprite.getVOffset(particle.getV1()));
        out.add("localUv", uv);
        var color = new JsonArray();
        color.add(particle.rCol); color.add(particle.gCol); color.add(particle.bCol); color.add(particle.alpha);
        out.add("color", color);
        var source = particle.sprite.contents();
        out.addProperty("staticSource", source.animatedTexture == null);
        // CPU source identity only, not a GPU readback claim.
        out.add("sourcePixels", GraphicsAuditTerrainParticleFixture.sourcePixels(source.originalImage));
        return out.toString();
    }
}
