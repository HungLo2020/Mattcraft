package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/** Capture-only inventory data and observation. No renderer or GPU access. */
public final class GraphicsAuditSpecialFoilFixture {
    private GraphicsAuditSpecialFoilFixture() {}
    public static List<ItemStack> items() {
        List<ItemStack> result=new ArrayList<>();
        result.add(new ItemStack(Items.APPLE));
        for(int slot=1;slot<9;slot++) {
            ItemStack stack=new ItemStack(slot%2==1 ? Items.CLOCK : Items.COMPASS);
            stack.set(DataComponents.ENCHANTMENT_GLINT_OVERRIDE,true);
            result.add(stack);
        }
        return List.copyOf(result);
    }
    public static String receipt(Minecraft minecraft) {
        if(minecraft.player==null) return "{\"complete\":false}";
        List<String> observed=new ArrayList<>();
        boolean complete=true;
        for(int slot=0;slot<9;slot++) {
            var stack=minecraft.player.getInventory().getItem(slot);
            var expected=slot==0 ? Items.APPLE : slot%2==1 ? Items.CLOCK : Items.COMPASS;
            complete &= stack.is(expected) && stack.getCount()==1 && stack.hasFoil()==(slot>0);
            observed.add("{\"item\":\""+stack.getItem().builtInRegistryHolder().key().location()
                +"\",\"count\":"+stack.getCount()+",\"foil\":"+stack.hasFoil()+"}");
        }
        return "{\"fixture\":\"gui-special-foil-v1\",\"items\":["+String.join(",",observed)
            +"],\"complete\":"+complete+"}";
    }
}
