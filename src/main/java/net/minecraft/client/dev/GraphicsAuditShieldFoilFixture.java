package net.minecraft.client.dev;

import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/** Opt-in inventory fixture: ordinary shield semantics, never renderer state. */
public final class GraphicsAuditShieldFoilFixture {
    private GraphicsAuditShieldFoilFixture() {}
    public static List<ItemStack> items(boolean foil) {
        var shield = new ItemStack(Items.SHIELD);
        shield.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE, foil);
        return List.of(shield, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY,
            ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY, ItemStack.EMPTY);
    }
    public static String receipt(Minecraft minecraft, boolean expectedFoil) {
        var player = minecraft.player;
        var stack = player == null ? ItemStack.EMPTY : player.getMainHandItem();
        int selected = player == null ? 0 : player.getInventory().getSelectedSlot() + 1;
        double speed = minecraft.options.glintSpeed().get();
        double strength = minecraft.options.glintStrength().get();
        boolean using = player != null && player.isUsingItem();
        boolean complete = selected == 1 && stack.is(Items.SHIELD) && stack.getCount() == 1
            && stack.hasFoil() == expectedFoil && !using && speed == 0.0 && strength == 0.5
            && stack.getOrDefault(DataComponents.BANNER_PATTERNS,
                net.minecraft.world.level.block.entity.BannerPatternLayers.EMPTY).layers().isEmpty()
            && !stack.has(DataComponents.BASE_COLOR);
        return "{\"fixture\":\"held-shield-v1\",\"selectedSlot\":" + selected
            + ",\"mainHand\":\"" + net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(stack.getItem())
            + "\",\"count\":" + stack.getCount() + ",\"foil\":" + stack.hasFoil()
            + ",\"usingItem\":" + using + ",\"speed\":" + speed + ",\"strength\":" + strength
            + ",\"complete\":" + complete + "}";
    }
}
