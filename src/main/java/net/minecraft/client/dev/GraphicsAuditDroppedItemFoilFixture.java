package net.minecraft.client.dev;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.lang.reflect.Field;
import net.minecraft.client.Minecraft;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.phys.Vec3;

/** Opt-in copied-world inputs only. Never reads or changes renderer/GPU state. */
public final class GraphicsAuditDroppedItemFoilFixture {
    public static final String PROPERTY = "mattmc.dev.graphicsAuditDroppedItemFoil";
    private static final int FIRST_ID = Integer.MIN_VALUE + 4100;
    static final float BOB_OFFSET = (float) Math.PI;
    private static final JsonObject[] observed = new JsonObject[2];
    private GraphicsAuditDroppedItemFoilFixture() {}

    public static int requestedCount() {
        int count = Integer.parseInt(System.getProperty(PROPERTY, "0"));
        if (count != 0 && count != 1 && count != 64)
            throw new IllegalArgumentException("dropped foil fixture count must be 0, 1 or 64");
        return count;
    }

    static Vec3 position(Vec3 eye, Vec3 look, int index) {
        Vec3 forward = look.lengthSqr() < .0001 ? new Vec3(0,0,1) : look.normalize();
        Vec3 side = new Vec3(forward.z,0,-forward.x);
        if (side.lengthSqr() < .0001) side = new Vec3(1,0,0);
        return eye.add(forward.scale(2)).add(side.normalize().scale(index == 0 ? 1.0 : .35))
            .add(0,-.35,0);
    }

    static void pinPhase(ItemEntity entity) {
        // bobOffs is randomly initialized and has no vanilla setter. This is
        // fixture entity DATA initialization, not a renderer override. Restrict
        // it to reserved fixture IDs and verify the actual value read by vanilla.
        if (entity.getId() < FIRST_ID || entity.getId() > FIRST_ID + 1)
            throw new IllegalArgumentException("refusing to mutate a non-fixture item");
        try {
            Field field = ItemEntity.class.getField("bobOffs");
            field.setAccessible(true);
            field.setFloat(entity, BOB_OFFSET);
        } catch (ReflectiveOperationException exception) {
            throw new IllegalStateException("cannot initialize deterministic item fixture phase", exception);
        }
        if (entity.bobOffs != BOB_OFFSET) throw new IllegalStateException("fixture phase write was not observed");
        entity.tickCount = 0;
    }

    public static void beforeRender(Minecraft minecraft) {
        int count = requestedCount();
        if (count == 0 || minecraft.level == null || minecraft.player == null) return;
        // Vanilla's frozen simulation exposes partial tick1 for non-player
        // entities. Pin the copied-world clock, not any renderer calculation.
        minecraft.level.tickRateManager().setFrozen(true);
        if (!(minecraft.getDeltaTracker() instanceof net.minecraft.client.DeltaTracker.Timer timer))
            throw new IllegalStateException("dropped fixture requires the ordinary game timer");
        timer.updateFrozenState(true);
        for (int index = 0; index < 2; index++) {
            int id = FIRST_ID + index;
            var existing = minecraft.level.getEntity(id);
            if (existing != null && !(existing instanceof ItemEntity))
                throw new IllegalStateException("dropped foil fixture ID collision");
            ItemEntity entity = (ItemEntity) existing;
            if (entity == null) {
                // Preserve the last rendered inputs while the presenter holds
                // its image for screenshot acknowledgement. beforeRender also
                // runs on those iterations, without a new extraction/draw.
                // A newly created entity must never inherit an old receipt.
                observed[index] = null;
                entity = new ItemEntity(EntityType.ITEM, minecraft.level);
                entity.setId(id);
                ItemStack stack = new ItemStack(index == 0 ? Items.DIAMOND : Items.STONE, count);
                stack.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
                entity.setItem(stack);
                entity.setNoGravity(true);
                entity.setNeverPickUp();
                entity.setUnlimitedLifetime();
                minecraft.level.addEntity(entity);
            }
            pinPhase(entity);
            entity.setDeltaMovement(Vec3.ZERO);
            entity.setPos(position(minecraft.player.getEyePosition(), minecraft.player.getLookAngle(), index));
            entity.setOldPosAndRot(entity.position(), 0, 0);
        }
    }

    /** Observes the ordinary extracted semantic input, without changing it. */
    public static void observe(ItemEntity entity, float age, float bob, int copies, int seed, int light) {
        if (requestedCount() == 0) return;
        int index = entity.getId() - FIRST_ID;
        if (index < 0 || index >= observed.length) return;
        JsonObject value = new JsonObject();
        value.addProperty("age", age);
        value.addProperty("bobOffset", bob);
        value.addProperty("copies", copies);
        value.addProperty("seed", seed);
        value.addProperty("light", light);
        observed[index] = value;
    }

    public static String receipt(Minecraft minecraft) {
        int count = requestedCount();
        if (count == 0) return "null";
        JsonObject result = new JsonObject();
        result.addProperty("fixture", "dropped-item-foil-v2");
        result.addProperty("stackCount", count);
        boolean complete = minecraft.level != null && minecraft.player != null;
        result.addProperty("frozenSimulation", complete && minecraft.level.tickRateManager().isFrozen());
        complete &= minecraft.level != null && minecraft.level.tickRateManager().isFrozen();
        JsonArray items = new JsonArray();
        if (complete) {
            for (int index = 0; index < 2; index++) {
                var found = minecraft.level.getEntity(FIRST_ID + index);
                if (!(found instanceof ItemEntity entity)) { complete = false; continue; }
                JsonObject item = new JsonObject();
                item.addProperty("item", index == 0 ? "minecraft:diamond" : "minecraft:stone");
                item.addProperty("count", entity.getItem().getCount());
                item.addProperty("foil", entity.getItem().hasFoil());
                item.addProperty("bobOffset", entity.bobOffs);
                item.addProperty("tickCount", entity.tickCount);
                JsonArray pos = new JsonArray();
                pos.add(entity.getX()); pos.add(entity.getY()); pos.add(entity.getZ());
                item.add("position", pos);
                item.add("extracted", observed[index]);
                items.add(item);
                complete &= entity.getItem().is(index == 0 ? Items.DIAMOND : Items.STONE)
                    && entity.getItem().getCount() == count && entity.getItem().hasFoil()
                    && entity.bobOffs == BOB_OFFSET && entity.tickCount == 0 && !entity.isRemoved()
                    && entity.isNoGravity() && entity.getDeltaMovement().lengthSqr() == 0;
                complete &= observed[index] != null;
            }
        }
        result.add("items", items);
        result.addProperty("complete", complete);
        return result.toString();
    }
}
