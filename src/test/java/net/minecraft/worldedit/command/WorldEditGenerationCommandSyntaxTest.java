package net.minecraft.worldedit.command;

import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class WorldEditGenerationCommandSyntaxTest {
    private static final Path SRC_MAIN_JAVA = Path.of("src/main/java");

    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void cylinderCommandsUseWorldEditStyleGreedyArguments() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/GenerationCommands.java"));

        assertTrue(source.contains("Commands.literal(\"/cyl\")"));
        assertTrue(source.contains("Commands.literal(\"/hcyl\")"));
        assertTrue(source.contains("Commands.argument(\"args\", StringArgumentType.greedyString())"));
        assertTrue(source.contains("cylinder(ctx, StringArgumentType.getString(ctx, \"args\"), false)"));
        assertTrue(source.contains("cylinder(ctx, StringArgumentType.getString(ctx, \"args\"), true)"));
        assertFalse(source.contains("private static int cylinder(CommandContext<CommandSourceStack> context, String blockName, int radius"));
        assertFalse(source.contains("StringArgumentType.getString(ctx, \"block\"),\n                            IntegerArgumentType.getInteger(ctx, \"radius\")"));
    }

    @Test
    void cylinderCommandsParsePatternsRadiiHeightAndHollowSwitch() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/GenerationCommands.java"));

        assertTrue(source.contains("Pattern pattern"));
        assertTrue(source.contains("BlockPatternParser.parse(tokens.get(0))"));
        assertTrue(source.contains("parseRadii(tokens.get(1))"));
        assertTrue(source.contains("height = 1"));
        assertTrue(source.contains("\"-h\".equals(token)"));
        assertTrue(source.contains("new CylinderArgs(pattern, Math.max(1, radii[0]), Math.max(1, radii[1]), height, hollow)"));
    }

    @Test
    void editSessionProvidesWorldEditStyleCylinderOperation() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/core/EditSession.java"));

        assertTrue(source.contains("public int makeCylinder(BlockVector3 pos, Pattern block, double radiusX, double radiusZ, int height, boolean filled)"));
        assertTrue(source.contains("radiusX = Math.max(1, radiusX) + 0.5"));
        assertTrue(source.contains("radiusZ = Math.max(1, radiusZ) + 0.5"));
        assertTrue(source.contains("if (height < 0)"));
        assertTrue(source.contains("lengthSq(nextXn, zn) <= 1 && lengthSq(xn, nextZn) <= 1"));
        assertTrue(source.contains("setBlock(BlockVector3 position, Pattern pattern)"));
    }

    @Test
    void cylinderBrushUsesSharedCylinderOperation() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/brush/CylinderBrush.java"));

        assertTrue(source.contains("editSession.makeCylinder(position, pattern, size, height, !hollow)"));
        assertFalse(source.contains("int radiusSquared = radius * radius"));
    }
}
