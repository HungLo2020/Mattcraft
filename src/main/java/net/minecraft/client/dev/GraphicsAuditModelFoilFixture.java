package net.minecraft.client.dev;

import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/** Opt-in copied-run inventory data and observed receipt; never renderer state. */
public final class GraphicsAuditModelFoilFixture {
    private GraphicsAuditModelFoilFixture() {}
    public static List<ItemStack> items() {
        var trident = new ItemStack(Items.TRIDENT);
        trident.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
        return List.of(trident, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY,
            ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY);
    }
    public static String receipt(Minecraft minecraft) {
        var player = minecraft.player;
        var stack = player == null ? ItemStack.EMPTY : player.getMainHandItem();
        int selected = player == null ? 0 : player.getInventory().getSelectedSlot() + 1;
        double speed = minecraft.options.glintSpeed().get();
        double strength = minecraft.options.glintStrength().get();
        boolean using = player != null && player.isUsingItem();
        boolean complete = selected == 1 && stack.is(Items.TRIDENT) && stack.getCount() == 1
            && stack.hasFoil() && !using && speed == 0.0 && strength == 0.5;
        return "{\"fixture\":\"held-trident-foil-v1\",\"selectedSlot\":" + selected
            + ",\"mainHand\":\"" + BuiltInRegistries.ITEM.getKey(stack.getItem())
            + "\",\"count\":" + stack.getCount() + ",\"foil\":" + stack.hasFoil()
            + ",\"usingItem\":" + using + ",\"speed\":" + speed + ",\"strength\":" + strength
            + ",\"complete\":" + complete + "}";
    }
}
