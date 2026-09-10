package net.minecraft.client.dev;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/** Opt-in shared semantic GUI workload; no backend, renderer or GPU state access. */
public final class GraphicsAuditGuiItemPlacementFixture {
    private static String lastReceipt = "null";
    private GraphicsAuditGuiItemPlacementFixture() {}

    public static boolean render(GuiGraphics gui, Player player, ItemStack item, int x, int y, int seed) {
        if (!Boolean.getBoolean("mattmc.dev.graphicsAuditGuiItemPlacement") || !item.is(Items.OAK_SLAB)) return false;
        int left=x, top=y-12, right=x+14, bottom=y+5;
        int originX=x-512; // Offscreen logical origin, moved onscreen by the GUI pose.
        // Scissor is established in the parent GUI space, before the item pose.
        gui.enableScissor(left,top,right,bottom);
        gui.pose().pushMatrix();
        try {
            gui.pose().translate(x+8,y+1).rotate(0.2F).scale(1.15F,0.9F).translate(-originX-8,-y-8);
            var pose=gui.pose();
            JsonObject receipt=new JsonObject();
            receipt.addProperty("schema","gui-item-placement-v1");
            receipt.addProperty("item","minecraft:oak_slab");
            receipt.addProperty("x",x); receipt.addProperty("y",y);
            receipt.addProperty("originX",originX);
            JsonArray matrix=new JsonArray();
            for (float value:new float[]{pose.m00(),pose.m01(),pose.m10(),pose.m11(),pose.m20(),pose.m21()})
                matrix.add(value);
            receipt.add("pose",matrix);
            JsonArray clip=new JsonArray();
            for (int value:new int[]{left,top,right-left,bottom-top}) clip.add(value);
            receipt.add("clip",clip);
            gui.renderItem(player,item,originX,y,seed);
            receipt.addProperty("submitted",true);
            lastReceipt=receipt.toString();
            return true;
        } finally {
            gui.pose().popMatrix();
            gui.disableScissor();
        }
    }

    public static String receipt() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditGuiItemPlacement") ? lastReceipt : "null";
    }
}
