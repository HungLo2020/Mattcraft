package net.minecraft.client.dev;

import java.util.List;
import java.util.stream.Collectors;
import net.minecraft.client.Minecraft;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponents;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.entity.BannerPattern;
import net.minecraft.world.level.block.entity.BannerPatternLayers;

/** Matched CPU item fixture only; no renderer or GPU state. */
public final class GraphicsAuditPatternedShieldFixture {
    private GraphicsAuditPatternedShieldFixture() {}
    private static BannerPatternLayers patterns() {
        return new BannerPatternLayers(List.of(
            new BannerPatternLayers.Layer(Holder.direct(new BannerPattern(
                ResourceLocation.withDefaultNamespace("cross"), "block.minecraft.banner.cross")), DyeColor.RED),
            new BannerPatternLayers.Layer(Holder.direct(new BannerPattern(
                ResourceLocation.withDefaultNamespace("border"), "block.minecraft.banner.border")), DyeColor.BLUE)));
    }
    public static List<ItemStack> items() {
        return items(false);
    }
    public static List<ItemStack> items(boolean foil) {
        var items = GraphicsAuditShieldFoilFixture.items(foil);
        items.getFirst().set(DataComponents.BASE_COLOR, DyeColor.YELLOW);
        items.getFirst().set(DataComponents.BANNER_PATTERNS, patterns());
        return items;
    }
    public static String receipt(Minecraft minecraft) {
        return receipt(minecraft,false);
    }
    public static String receipt(Minecraft minecraft, boolean expectedFoil) {
        var player = minecraft.player;
        var stack = player == null ? ItemStack.EMPTY : player.getMainHandItem();
        int selected = player == null ? 0 : player.getInventory().getSelectedSlot()+1;
        var base = stack.get(DataComponents.BASE_COLOR);
        var actual = stack.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY);
        boolean using = player != null && player.isUsingItem();
        String layers = actual.layers().stream().map(layer -> layer.pattern().value().assetId()
            + ":" + layer.color().getName()).collect(Collectors.joining(","));
        boolean complete = selected == 1 && stack.is(Items.SHIELD) && stack.getCount() == 1
            && stack.hasFoil() == expectedFoil && !using && base == DyeColor.YELLOW && actual.equals(patterns())
            && (!expectedFoil || (minecraft.options.glintSpeed().get() == 0.0 && minecraft.options.glintStrength().get() == 0.5));
        return "{\"fixture\":\"" + (expectedFoil ? "held-shield-patterns-foil-v1" : "held-shield-patterns-v1") + "\",\"selectedSlot\":" + selected
            + ",\"mainHand\":\"" + net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(stack.getItem())
            + "\",\"count\":" + stack.getCount() + ",\"foil\":" + stack.hasFoil()
            + ",\"usingItem\":" + using + ",\"baseColor\":\"" + (base == null ? "none" : base.getName())
            + "\",\"patterns\":\"" + layers + "\",\"complete\":" + complete
            + (expectedFoil ? ",\"speed\":" + minecraft.options.glintSpeed().get() + ",\"strength\":" + minecraft.options.glintStrength().get() : "") + "}";
    }
}
