package net.minecraft.worldedit.pattern;

import net.minecraft.core.Direction;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.StairBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Half;
import net.minecraft.worldedit.mask.Mask;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

class BlockPatternParserTest {
    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void parsesSingleBlockPattern() {
        Pattern pattern = BlockPatternParser.parse("stone");

        BlockState state = pattern.apply(net.minecraft.worldedit.math.BlockVector3.at(0, 0, 0));

        assertEquals(Blocks.STONE.defaultBlockState(), state);
    }

    @Test
    void parsesEvenRandomPattern() {
        Pattern pattern = BlockPatternParser.parse("cobblestone,diorite");

        RandomPattern randomPattern = assertInstanceOf(RandomPattern.class, pattern);
        assertEquals(2, randomPattern.getEntryCount());
        assertEquals(2.0, randomPattern.getTotalWeight(), 0.0001);
    }

    @Test
    void parsesWeightedRandomPatternAsRelativeWeights() {
        Pattern pattern = BlockPatternParser.parse("50%cobblestone,10%diorite");

        RandomPattern randomPattern = assertInstanceOf(RandomPattern.class, pattern);
        assertEquals(2, randomPattern.getEntryCount());
        assertEquals(60.0, randomPattern.getTotalWeight(), 0.0001);
    }

    @Test
    void rejectsSingleWeightedPattern() {
        assertThrows(IllegalArgumentException.class, () -> BlockPatternParser.parse("5%dirt"));
    }

    @Test
    void parsesMultipleInputMaskAndWeightedOutputPattern() {
        BlockPatternParser.ReplacementPatterns replacement =
            BlockPatternParser.parseReplacementPatterns("stone,dirt 70%cobblestone,30%diorite");

        assertTrue(replacement.from().test(Blocks.STONE.defaultBlockState()));
        assertTrue(replacement.from().test(Blocks.DIRT.defaultBlockState()));
        assertFalse(replacement.from().test(Blocks.GRASS_BLOCK.defaultBlockState()));

        RandomPattern randomPattern = assertInstanceOf(RandomPattern.class, replacement.to());
        assertEquals(2, randomPattern.getEntryCount());
        assertEquals(100.0, randomPattern.getTotalWeight(), 0.0001);
    }

    @Test
    void parsesBlockStatesWithoutSplittingPropertyCommas() {
        Pattern pattern = BlockPatternParser.parse("oak_stairs[facing=east,half=top],stone");

        RandomPattern randomPattern = assertInstanceOf(RandomPattern.class, pattern);
        assertEquals(2, randomPattern.getEntryCount());

        BlockState stairState = BlockPatternParser.parseSingleBlockState("oak_stairs[facing=east,half=top]");
        assertEquals(Direction.EAST, stairState.getValue(StairBlock.FACING));
        assertEquals(Half.TOP, stairState.getValue(StairBlock.HALF));
    }

    @Test
    void parsesMultipleInputBlockStatesWithoutSplittingPropertyCommas() {
        Mask mask = BlockPatternParser.parseMask("oak_stairs[facing=east,half=top],stone");

        assertTrue(mask.test(BlockPatternParser.parseSingleBlockState("oak_stairs[facing=east,half=top]")));
        assertFalse(mask.test(BlockPatternParser.parseSingleBlockState("oak_stairs[facing=north,half=bottom]")));
        assertTrue(mask.test(Blocks.STONE.defaultBlockState()));
        assertFalse(mask.test(Blocks.DIORITE.defaultBlockState()));
    }

    @Test
    void masksWithoutPropertiesMatchEveryStateOfThatBlockType() {
        Mask mask = BlockPatternParser.parseMask("oak_stairs");

        assertTrue(mask.test(BlockPatternParser.parseSingleBlockState("oak_stairs[facing=east,half=top]")));
        assertTrue(mask.test(BlockPatternParser.parseSingleBlockState("oak_stairs[facing=north,half=bottom]")));
        assertFalse(mask.test(Blocks.STONE.defaultBlockState()));
    }
}
