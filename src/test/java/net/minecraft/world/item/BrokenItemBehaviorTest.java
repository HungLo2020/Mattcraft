package net.minecraft.world.item;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.level.block.Blocks;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

class BrokenItemBehaviorTest {
	private static final Path SRC_MAIN_JAVA = Path.of("src/main/java");

	@BeforeAll
	static void bootstrap() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void brokenToolsMineLikeEmptyHandAndDoNotCountForDrops() {
		ItemStack pickaxe = new ItemStack(Items.DIAMOND_PICKAXE);
		pickaxe.setDamageValue(pickaxe.getMaxDamage());

		assertTrue(pickaxe.isBroken());
		assertEquals(1.0F, pickaxe.getDestroySpeed(Blocks.STONE.defaultBlockState()));
		assertFalse(pickaxe.isCorrectToolForDrops(Blocks.STONE.defaultBlockState()));
	}

	@Test
	void brokenWeaponsDoNotRunWeaponUseOrHitLogic() {
		ItemStack sword = new ItemStack(Items.DIAMOND_SWORD);
		sword.setDamageValue(sword.getMaxDamage());

		assertTrue(sword.isBroken());
		assertEquals(InteractionResult.PASS, sword.use(null, null, null));
		assertFalse(sword.hurtEnemy(null, null));
	}

	@Test
	void brokenBowsAndCrossbowsDoNotStartUsing() {
		ItemStack bow = new ItemStack(Items.BOW);
		ItemStack crossbow = new ItemStack(Items.CROSSBOW);
		bow.setDamageValue(bow.getMaxDamage());
		crossbow.setDamageValue(crossbow.getMaxDamage());

		assertEquals(InteractionResult.PASS, bow.use(null, null, null));
		assertEquals(InteractionResult.PASS, crossbow.use(null, null, null));
	}

	@Test
	void breakEventNoLongerShrinksTheStackAndOnlyFiresOnce() throws IOException {
		String itemStack = read("net/minecraft/world/item/ItemStack.java");

		assertFalse(itemStack.contains("this.shrink(1);\n\t\t\tconsumer.accept(item);"));
		assertTrue(itemStack.contains("boolean bl = this.isBroken();"));
		assertTrue(itemStack.contains("if (!bl && this.isBroken())"));
	}

	@Test
	void directEntityToolInteractionsRejectBrokenItems() throws IOException {
		assertTrue(read("net/minecraft/world/entity/Entity.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/animal/sheep/Sheep.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/animal/MushroomCow.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/animal/SnowGolem.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/monster/Bogged.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/animal/coppergolem/CopperGolem.java").contains("itemStack.is(Items.SHEARS) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/animal/coppergolem/CopperGolem.java").contains("itemStack.is(ItemTags.AXES) && !itemStack.isBroken()"));
		assertTrue(read("net/minecraft/world/entity/monster/Creeper.java").contains("itemStack.is(ItemTags.CREEPER_IGNITERS) && !itemStack.isBroken()"));
	}

	private static String read(String path) throws IOException {
		return Files.readString(SRC_MAIN_JAVA.resolve(path));
	}
}
