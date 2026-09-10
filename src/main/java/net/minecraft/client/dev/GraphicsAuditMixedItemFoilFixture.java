package net.minecraft.client.dev;

import java.util.Locale;
import net.minecraft.client.Minecraft;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.entity.Display;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.phys.Vec3;

/** Opt-in copied-world input. Uses vanilla entity data, never renderer state. */
public final class GraphicsAuditMixedItemFoilFixture {
    private static final int ID = Integer.MIN_VALUE + 4097;
    private GraphicsAuditMixedItemFoilFixture() {}
    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditMixedItemFoil");
    }
    public static Vec3 position(Vec3 eye, Vec3 look) {
        Vec3 forward = look.lengthSqr() < .0001 ? new Vec3(0,0,1) : look.normalize();
        Vec3 right = new Vec3(forward.z,0,-forward.x);
        if (right.lengthSqr() < .0001) right = new Vec3(1,0,0);
        return eye.add(forward.scale(4)).add(right.normalize().scale(1.3)).add(0,-.4,0);
    }
    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        var existing = minecraft.level.getEntity(ID);
        if (existing != null) {
            if (!(existing instanceof Display.ItemDisplay)) throw new IllegalStateException("mixed foil fixture entity ID collision");
            return;
        }
        var item = new ItemStack(Items.DIAMOND);
        item.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE,true);
        var display = new Display.ItemDisplay(EntityType.ITEM_DISPLAY,minecraft.level);
        display.setId(ID);
        display.setPos(position(minecraft.player.getEyePosition(),minecraft.player.getLookAngle()));
        display.setOldPosAndRot(display.position(),0,0);
        display.setItemStack(item);
        display.setItemTransform(ItemDisplayContext.GROUND);
        display.setBillboardConstraints(Display.BillboardConstraints.CENTER);
        display.setTransformation(new net.math.Transformation(new org.joml.Vector3f(),
            new org.joml.Quaternionf(0,1,0,0),new org.joml.Vector3f(2),new org.joml.Quaternionf()));
        display.setViewRange(16);
        display.setWidth(4);
        display.setHeight(4);
        minecraft.level.addEntity(display);
    }
    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        if (minecraft.level == null || !(minecraft.level.getEntity(ID) instanceof Display.ItemDisplay display))
            return "{\"fixture\":\"mixed-item-foil-v2\",\"complete\":false}";
        boolean complete = !display.isRemoved() && display.getItemStack().is(Items.DIAMOND)
            && display.getItemStack().getCount() == 1 && display.getItemStack().hasFoil()
            && display.getItemTransform() == ItemDisplayContext.GROUND;
        return String.format(Locale.ROOT,
            "{\"fixture\":\"mixed-item-foil-v2\",\"item\":\"minecraft:diamond\",\"context\":\"ground\",\"position\":[%.6f,%.6f,%.6f],\"foil\":%s,\"complete\":%s}",
            display.getX(),display.getY(),display.getZ(),display.getItemStack().hasFoil(),complete);
    }
}
