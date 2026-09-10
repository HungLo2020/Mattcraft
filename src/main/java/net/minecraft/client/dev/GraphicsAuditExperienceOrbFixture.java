package net.minecraft.client.dev;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.entity.state.ExperienceOrbRenderState;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.phys.Vec3;

/** Shared opt-in CPU fixture inputs. No resources, draw calls or backend state. */
public final class GraphicsAuditExperienceOrbFixture {
    static final String PROPERTY = "mattmc.dev.graphicsAuditNativeOrb";
    private static final int ENTITY_ID = Integer.MAX_VALUE - 113;
    private static final int[] VALUES = {1, 3, 7, 17, 37, 73, 149, 307, 617, 1237, 2477};
    private static ClientLevel installedLevel;
    private static ExperienceOrb orb;
    private static int extractedLight;
    private static int extractedIcon;
    private static float extractedAge;
    private static long extractions;
    private static ExperienceOrbRenderState ownedState;
    private static long submits;
    private static long callbacks;
    private static float[] submittedPose;
    private static float[] callbackPose;
    private static double[] extractedPosition;
    private static int fixturePoseIndex;
    private static final Float[] observedPoseAges = new Float[5];
    private static Vec3 initialEye;
    private static Vec3 initialPosition;
    private static final Vec3[] configuredPosePositions = new Vec3[5];
    private static final Vec3[] observedPosePositions = new Vec3[5];
    private static net.minecraft.core.BlockPos occluderPosition;
    private static java.util.concurrent.CompletableFuture<Void> occluderPlacement;

    private GraphicsAuditExperienceOrbFixture() {}

    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.deterministicCameraCapture")
            && System.getProperty(PROPERTY) != null;
    }

    static int valueForIcon(int icon) {
        if (icon < 0 || icon >= VALUES.length) throw new IllegalArgumentException("orb fixture icon outside 0..10");
        return VALUES[icon];
    }

    static float checkedAge(String value) {
        float age = Float.parseFloat(value);
        if (!Float.isFinite(age) || age < 0 || age > 20000)
            throw new IllegalArgumentException("invalid orb fixture age");
        return age;
    }

    static Vec3 position(Vec3 eye, Vec3 look) {
        if (!Double.isFinite(eye.x) || !Double.isFinite(eye.y) || !Double.isFinite(eye.z)
                || !Double.isFinite(look.lengthSqr()) || look.lengthSqr() < 0.0001)
            throw new IllegalArgumentException("invalid orb fixture camera");
        Vec3 forward = look.normalize();
        Vec3 side = forward.cross(new Vec3(0, 1, 0)).normalize();
        return eye.add(forward.scale(3)).add(side.scale(0.6));
    }

    private static int icon() { return Integer.parseInt(System.getProperty(PROPERTY)); }
    private static boolean hidden() { return Boolean.getBoolean("mattmc.dev.graphicsAuditNativeOrbHidden"); }
    static float ageForPose(String base, String step, int index) {
        if (index < 0 || index >= 5) throw new IllegalArgumentException("orb fixture pose outside 0..4");
        float value = checkedAge(base) + checkedAge(step) * index;
        return checkedAge(Float.toString(value));
    }

    static boolean isCapturePose(int index) {
        if (index < 0 || index > 5) throw new IllegalArgumentException("invalid orb capture lifecycle index");
        // The capture coordinator restores the initial camera at terminal index5.
        // That is not a sixth sample and must not mutate the last fixture receipt.
        return index < 5;
    }

    private static float age() {
        return ageForPose(System.getProperty("mattmc.dev.graphicsAuditNativeOrbAge", "4"),
            System.getProperty("mattmc.dev.graphicsAuditNativeOrbAgeStep", "0"), fixturePoseIndex);
    }

    static Vec3 positionForPose(Vec3 eye, Vec3 initial, int index, boolean distanceSequence) {
        if (!isCapturePose(index)) throw new IllegalArgumentException("terminal pose has no orb placement");
        return distanceSequence ? eye.add(initial.subtract(eye).scale((3.0 + index) / 3.0)) : initial;
    }

    private static boolean distanceSequence() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditNativeOrbDistanceSequence");
    }

    private static boolean occluderRequested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditNativeOrbOccluder");
    }

    static net.minecraft.core.BlockPos checkedOccluderPosition(Vec3 origin, boolean moving) {
        if (moving) throw new IllegalArgumentException("orb occlusion requires stationary placement");
        if (!Double.isFinite(origin.x) || !Double.isFinite(origin.y) || !Double.isFinite(origin.z))
            throw new IllegalArgumentException("nonfinite orb occluder position");
        return net.minecraft.core.BlockPos.containing(origin);
    }

    public static void install(Minecraft minecraft, int poseIndex) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        if (!isCapturePose(poseIndex)) return;
        ageForPose(System.getProperty("mattmc.dev.graphicsAuditNativeOrbAge", "4"),
            System.getProperty("mattmc.dev.graphicsAuditNativeOrbAgeStep", "0"), poseIndex);
        fixturePoseIndex = poseIndex;
        if (installedLevel == minecraft.level && orb != null) {
            if (configuredPosePositions[poseIndex] == null) {
                Vec3 position = positionForPose(initialEye, initialPosition, poseIndex, distanceSequence());
                orb.setPos(position);
                orb.setOldPosAndRot();
                configuredPosePositions[poseIndex] = position;
            }
            return;
        }
        int value = valueForIcon(icon());
        age(); // Reject invalid fixture inputs before modifying the client level.
        if (minecraft.level.getEntity(ENTITY_ID) != null)
            throw new IllegalStateException("orb fixture entity ID collision");
        Vec3 origin = position(minecraft.player.getEyePosition(), minecraft.player.getLookAngle());
        occluderPosition = null;
        occluderPlacement = null;
        if (occluderRequested()) {
            var block = checkedOccluderPosition(origin, distanceSequence());
            var server = minecraft.getSingleplayerServer();
            if (server == null) throw new IllegalStateException("orb occluder requires an isolated integrated server");
            var dimension = minecraft.level.dimension();
            occluderPosition = block;
            // Use authoritative world mutation and ordinary block/light packets.
            // A client-only block can be contradicted by later server light data.
            occluderPlacement = server.submit(() -> {
                var serverLevel = server.getLevel(dimension);
                if (serverLevel == null || !serverLevel.getBlockState(block).isAir())
                    throw new IllegalStateException("orb occlusion fixture refuses to overwrite a non-air block");
                serverLevel.setBlock(block, net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(), 3);
            });
        }
        initialEye = minecraft.player.getEyePosition();
        initialPosition = origin;
        // Only this synthetic test entity is held stationary. Ordinary entities
        // keep their unmodified simulation and renderer in both repositories.
        ExperienceOrb created = new ExperienceOrb(minecraft.level, origin, Vec3.ZERO, value) {
            @Override public void tick() { setOldPosAndRot(); }
        };
        created.setId(ENTITY_ID);
        created.setNoGravity(true);
        created.setDeltaMovement(Vec3.ZERO);
        created.setOldPosAndRot();
        installedLevel = minecraft.level;
        orb = created;
        extractions = 0;
        ownedState = null;
        submits = callbacks = 0;
        submittedPose = callbackPose = null;
        extractedPosition = null;
        java.util.Arrays.fill(observedPoseAges, null);
        java.util.Arrays.fill(configuredPosePositions, null);
        java.util.Arrays.fill(observedPosePositions, null);
        configuredPosePositions[poseIndex] = origin;
        // A hidden control must submit no orb: invisibility alone does not
        // suppress ExperienceOrbRenderer's custom geometry.
        if (!hidden()) minecraft.level.addEntity(created);
    }

    /** Fixed temporal input for the owned synthetic entity only; normal state is untouched. */
    public static void configureRenderState(ExperienceOrb source, ExperienceOrbRenderState state) {
        if (!requested() || orb == null || source != orb) return;
        state.ageInTicks = age();
        ownedState = state;
        extractedPosition = new double[]{state.x, state.y, state.z};
        observedPosePositions[fixturePoseIndex] = new Vec3(state.x, state.y, state.z);
        extractedAge = state.ageInTicks;
        observedPoseAges[fixturePoseIndex] = extractedAge;
        extractedIcon = state.icon;
        extractedLight = state.lightCoords;
        extractions++;
    }

    /** Observation only: no mutation of the supplied pose or rendering state. */
    public static void observeSubmit(ExperienceOrbRenderState state, org.joml.Matrix4f pose) {
        if (!requested() || ownedState == null || state != ownedState) return;
        submits++;
        submittedPose = pose.get(new float[16]);
    }

    public static void observeCallback(ExperienceOrbRenderState state, org.joml.Matrix4f pose) {
        if (!requested() || ownedState == null || state != ownedState) return;
        callbacks++;
        callbackPose = pose.get(new float[16]);
    }

    private static JsonArray matrix(float[] values) {
        JsonArray out = new JsonArray();
        if (values != null) for (float value : values) out.add(value);
        return out;
    }

    public static boolean hasSubmittedFixture() { return requested() && submits > 0; }

    static boolean settledOccluder(boolean placed, boolean present, boolean pendingLight, int skyLight) {
        return placed && present && !pendingLight && skyLight == 0;
    }

    public static boolean ready(Minecraft minecraft) {
        if (!requested() || !occluderRequested()) return true;
        if (occluderPlacement == null || minecraft.level == null || occluderPosition == null) return false;
        if (occluderPlacement.isDone()) occluderPlacement.join();
        var lights = minecraft.level.getChunkSource().getLightEngine();
        return settledOccluder(occluderPlacement.isDone(),
            minecraft.level.getBlockState(occluderPosition).is(net.minecraft.world.level.block.Blocks.STONE),
            minecraft.level.hasPendingLightUpdates() || lights.hasLightWork(),
            minecraft.level.getBrightness(net.minecraft.world.level.LightLayer.SKY, occluderPosition));
    }

    private static JsonArray positions(Vec3[] values) {
        JsonArray result = new JsonArray();
        for (Vec3 value : values) {
            if (value == null) { result.add(com.google.gson.JsonNull.INSTANCE); continue; }
            JsonArray position = new JsonArray();
            position.add(value.x); position.add(value.y); position.add(value.z);
            result.add(position);
        }
        return result;
    }

    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        JsonObject out = new JsonObject();
        out.addProperty("fixture", "native-experience-orb-v1");
        out.addProperty("icon", icon());
        out.addProperty("age", age());
        out.addProperty("ageBase", checkedAge(System.getProperty("mattmc.dev.graphicsAuditNativeOrbAge", "4")));
        out.addProperty("ageStep", checkedAge(System.getProperty("mattmc.dev.graphicsAuditNativeOrbAgeStep", "0")));
        out.addProperty("poseIndex", fixturePoseIndex);
        JsonArray ages = new JsonArray();
        for (Float value : observedPoseAges) ages.add(value);
        out.add("observedPoseAges", ages);
        out.addProperty("distanceSequence", distanceSequence());
        out.addProperty("occluderRequested", occluderRequested());
        out.addProperty("occluderReady", ready(minecraft));
        JsonObject occluder = new JsonObject();
        if (occluderPosition != null && minecraft.level != null) {
            occluder.addProperty("x", occluderPosition.getX());
            occluder.addProperty("y", occluderPosition.getY());
            occluder.addProperty("z", occluderPosition.getZ());
            occluder.addProperty("block", net.minecraft.core.registries.BuiltInRegistries.BLOCK
                .getKey(minecraft.level.getBlockState(occluderPosition).getBlock()).toString());
        }
        out.add("occluder", occluder);
        out.add("configuredPosePositions", positions(configuredPosePositions));
        out.add("observedPosePositions", positions(observedPosePositions));
        out.addProperty("hidden", hidden());
        boolean installed = installedLevel == minecraft.level && orb != null && minecraft.level != null;
        boolean present = installed && minecraft.level.getEntity(ENTITY_ID) == orb;
        out.addProperty("complete", installed && !orb.isRemoved()
            && (hidden() ? minecraft.level.getEntity(ENTITY_ID) == null && extractions == 0
                : present && extractions > 0 && extractedIcon == icon() && extractedAge == age()));
        out.addProperty("present", present);
        out.addProperty("extractions", extractions);
        out.addProperty("submits", submits);
        out.addProperty("callbacks", callbacks);
        out.add("submittedPose", matrix(submittedPose));
        out.add("callbackPose", matrix(callbackPose));
        JsonArray extracted = new JsonArray();
        if (extractedPosition != null) for (double value : extractedPosition) extracted.add(value);
        out.add("extractedPosition", extracted);
        if (installed) {
            out.addProperty("entityId", orb.getId());
            out.addProperty("value", orb.getValue());
            JsonArray position = new JsonArray();
            position.add(orb.getX()); position.add(orb.getY()); position.add(orb.getZ());
            out.add("position", position);
        }
        if (extractions > 0) {
            out.addProperty("extractedIcon", extractedIcon);
            out.addProperty("extractedAge", extractedAge);
            out.addProperty("extractedLight", extractedLight);
        }
        return out.toString();
    }
}
