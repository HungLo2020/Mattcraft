package net.minecraft.client.dev;

import com.google.gson.JsonObject;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.animal.Cow;

/** Opt-in world fixture only; uses vanilla server metadata, never renderer state. */
public final class GraphicsAuditCowOutlineFixture {
    private static final String GLOW = "mattmc.dev.graphicsAuditCowGlowing";
    private static final String INVISIBLE = "mattmc.dev.graphicsAuditCowInvisible";
    private static JsonObject lastModelObservation;

    /** Copies CPU inputs only; never changes animation, transforms or GPU state. */
    public static void observeModel(Object model, Object state, org.joml.Matrix4fc pose) {
        if (!requested() || !(model instanceof net.minecraft.client.model.CowModel cowModel)
            || !(state instanceof net.minecraft.client.renderer.entity.state.CowRenderState cowState)) return;
        JsonObject observation = new JsonObject();
        observation.addProperty("x", cowState.x);
        observation.addProperty("y", cowState.y);
        observation.addProperty("z", cowState.z);
        observation.addProperty("bodyRot", cowState.bodyRot);
        observation.addProperty("yRot", cowState.yRot);
        observation.addProperty("xRot", cowState.xRot);
        observation.addProperty("walkAnimationPos", cowState.walkAnimationPos);
        observation.addProperty("walkAnimationSpeed", cowState.walkAnimationSpeed);
        observation.addProperty("scale", cowState.scale);
        observation.addProperty("ageScale", cowState.ageScale);
        float[] transform = new float[16];
        pose.get(transform);
        observation.add("pose", numbers(transform));
        JsonObject parts = new JsonObject();
        observePart(parts, "root", cowModel.root());
        observation.add("parts", parts);
        lastModelObservation = observation;
    }

    private static com.google.gson.JsonArray numbers(float... values) {
        com.google.gson.JsonArray result = new com.google.gson.JsonArray();
        for (float value : values) result.add(value);
        return result;
    }

    private static void observePart(JsonObject parts, String path, net.minecraft.client.model.geom.ModelPart part) {
        parts.add(path, numbers(part.x, part.y, part.z, part.xRot, part.yRot, part.zRot,
            part.xScale, part.yScale, part.zScale));
        part.children.forEach((name, child) -> observePart(parts, path + "/" + name, child));
    }

    static boolean requested() { return Boolean.getBoolean(GLOW); }

    static void configure(Cow cow) {
        if (!requested()) return;
        cow.setGlowingTag(true);
        if (Boolean.getBoolean(INVISIBLE)) {
            cow.addEffect(new net.minecraft.world.effect.MobEffectInstance(
                net.minecraft.world.effect.MobEffects.INVISIBILITY,
                net.minecraft.world.effect.MobEffectInstance.INFINITE_DURATION, 0, false, false));
        }
        cow.setInvisible(Boolean.getBoolean(INVISIBLE));
        // LivingEntity's constructor randomizes head yaw. No-AI alone does
        // not remove that input: client body rotation follows the head.
        // Set an identical server-side fixture pose before normal replication.
        cow.setYRot(0.0F);
        cow.yRotO = 0.0F;
        cow.setXRot(0.0F);
        cow.xRotO = 0.0F;
        cow.setYHeadRot(0.0F);
        cow.yHeadRotO = 0.0F;
        cow.setYBodyRot(0.0F);
        cow.yBodyRotO = 0.0F;
    }

    static boolean ready(Entity entity) {
        return !requested() || entity instanceof Cow cow
            && entity.isCurrentlyGlowing()
            && entity.isInvisible() == Boolean.getBoolean(INVISIBLE)
            && cow.getYRot() == 0.0F && cow.getXRot() == 0.0F
            && cow.getYHeadRot() == 0.0F && cow.yBodyRot == 0.0F;
    }

    static JsonObject receipt(Entity entity) {
        JsonObject result = new JsonObject();
        result.addProperty("requested", requested());
        result.addProperty("invisibleRequested", Boolean.getBoolean(INVISIBLE));
        result.addProperty("ready", ready(entity));
        result.addProperty("entityPresent", entity != null);
        if (lastModelObservation != null) result.add("modelObservation", lastModelObservation.deepCopy());
        if (entity != null) {
            result.addProperty("glowing", entity.isCurrentlyGlowing());
            result.addProperty("invisible", entity.isInvisible());
            result.addProperty("x", entity.getX());
            result.addProperty("y", entity.getY());
            result.addProperty("z", entity.getZ());
        }
        return result;
    }

    private GraphicsAuditCowOutlineFixture() {}
}
