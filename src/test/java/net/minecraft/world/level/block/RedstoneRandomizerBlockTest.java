package net.minecraft.world.level.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.RedstoneRandomizerBlock.OutputSide;
import net.minecraft.world.level.block.state.BlockState;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class RedstoneRandomizerBlockTest {
	private static final Path SRC_MAIN_JAVA = Path.of("src/main/java");
	private static final Path ASSETS = Path.of("src/main/resources/assets/minecraft");
	private static final Path DATA = Path.of("src/main/resources/data/minecraft");

	@BeforeAll
	static void bootstrapMinecraft() {
		SharedConstants.tryDetectVersion();
		Bootstrap.bootStrap();
	}

	@Test
	void poweredStateEmitsOnlyTheSelectedSide() {
		RedstoneRandomizerBlock block = (RedstoneRandomizerBlock)Blocks.REDSTONE_RANDOMIZER;
		BlockState leftState = block.defaultBlockState()
			.setValue(DiodeBlock.FACING, Direction.NORTH)
			.setValue(DiodeBlock.POWERED, true)
			.setValue(RedstoneRandomizerBlock.OUTPUT_SIDE, OutputSide.LEFT);
		BlockState rightState = leftState.setValue(RedstoneRandomizerBlock.OUTPUT_SIDE, OutputSide.RIGHT);

		assertEquals(Direction.WEST, block.getPhysicalOutputDirection(leftState));
		assertEquals(Direction.EAST, block.getPhysicalOutputDirection(rightState));
		assertEquals(15, leftState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.EAST));
		assertEquals(0, leftState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.WEST));
		assertEquals(0, leftState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.NORTH));
		assertEquals(0, leftState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.SOUTH));

		assertEquals(15, rightState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.WEST));
		assertEquals(0, rightState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.EAST));
		assertEquals(0, rightState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.NORTH));
		assertEquals(0, rightState.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, Direction.SOUTH));
	}

	@Test
	void unpoweredStateEmitsNoSignal() {
		RedstoneRandomizerBlock block = (RedstoneRandomizerBlock)Blocks.REDSTONE_RANDOMIZER;
		BlockState state = block.defaultBlockState()
			.setValue(DiodeBlock.FACING, Direction.SOUTH)
			.setValue(DiodeBlock.POWERED, false)
			.setValue(RedstoneRandomizerBlock.OUTPUT_SIDE, OutputSide.LEFT);

		for (Direction direction : Direction.values()) {
			assertEquals(0, state.getSignal(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, direction));
		}
	}

	@Test
	void redstoneRandomizerIsRegisteredNextToComparator() throws IOException {
		String blocks = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/world/level/block/Blocks.java"));
		String items = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/world/item/Items.java"));
		String blockTypes = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/world/level/block/BlockTypes.java"));
		String tabs = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/world/item/CreativeModeTabs.java"));

		assertTrue(blocks.contains("public static final Block REDSTONE_RANDOMIZER"));
		assertTrue(blocks.contains("\"redstone_randomizer\", RedstoneRandomizerBlock::new"));
		assertTrue(items.contains("public static final Item REDSTONE_RANDOMIZER = registerBlock(Blocks.REDSTONE_RANDOMIZER);"));
		assertTrue(blockTypes.contains("Registry.register(registry, \"redstone_randomizer\", RedstoneRandomizerBlock.CODEC);"));
		assertTrue(tabs.indexOf("output.accept(Items.REDSTONE_RANDOMIZER)") > tabs.indexOf("output.accept(Items.COMPARATOR)"));
	}

	@Test
	void redstoneRandomizerHasLanguageAndModelResources() throws IOException {
		String language = Files.readString(ASSETS.resolve("lang/en_us.json"));
		String blockState = Files.readString(ASSETS.resolve("blockstates/redstone_randomizer.json"));
		String blockModel = Files.readString(ASSETS.resolve("models/block/redstone_randomizer.json"));
		String poweredBlockModel = Files.readString(ASSETS.resolve("models/block/redstone_randomizer_on.json"));
		String itemDefinition = Files.readString(ASSETS.resolve("items/redstone_randomizer.json"));
		String itemModel = Files.readString(ASSETS.resolve("models/item/redstone_randomizer.json"));
		String lootTable = Files.readString(DATA.resolve("loot_table/blocks/redstone_randomizer.json"));

		assertTrue(language.contains("\"block.minecraft.redstone_randomizer\": \"Redstone Randomizer\""));
		assertTrue(blockState.contains("facing=north,output=left,powered=false"));
		assertTrue(blockState.contains("facing=south,output=right,powered=true"));
		assertTrue(blockModel.contains("\"top\": \"block/redstone_randomizer\""));
		assertTrue(poweredBlockModel.contains("\"top\": \"block/redstone_randomizer_on\""));
		assertFalse(blockModel.contains("block/comparator"));
		assertFalse(poweredBlockModel.contains("block/comparator"));
		assertTrue(itemDefinition.contains("\"model\": \"minecraft:item/redstone_randomizer\""));
		assertTrue(itemModel.contains("\"layer0\": \"minecraft:item/redstone_randomizer\""));
		assertTrue(lootTable.contains("\"name\": \"minecraft:redstone_randomizer\""));
	}

	@Test
	void redstoneRandomizerUsesDuplicatedComparatorTextures() throws IOException {
		byte[] comparatorBlock = Files.readAllBytes(ASSETS.resolve("textures/block/comparator.png"));
		byte[] randomizerBlock = Files.readAllBytes(ASSETS.resolve("textures/block/redstone_randomizer.png"));
		byte[] comparatorBlockOn = Files.readAllBytes(ASSETS.resolve("textures/block/comparator_on.png"));
		byte[] randomizerBlockOn = Files.readAllBytes(ASSETS.resolve("textures/block/redstone_randomizer_on.png"));
		byte[] comparatorItem = Files.readAllBytes(ASSETS.resolve("textures/item/comparator.png"));
		byte[] randomizerItem = Files.readAllBytes(ASSETS.resolve("textures/item/redstone_randomizer.png"));

		assertTrue(Arrays.equals(comparatorBlock, randomizerBlock));
		assertTrue(Arrays.equals(comparatorBlockOn, randomizerBlockOn));
		assertTrue(Arrays.equals(comparatorItem, randomizerItem));
	}
}
