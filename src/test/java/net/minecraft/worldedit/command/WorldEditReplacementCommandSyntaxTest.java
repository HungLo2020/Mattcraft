package net.minecraft.worldedit.command;

import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import net.minecraft.SharedConstants;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.synchronization.ArgumentTypeInfos;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.worldedit.command.argument.WorldEditMaskArgument;
import net.minecraft.worldedit.command.argument.WorldEditPatternArgument;
import net.minecraft.worldedit.command.argument.WorldEditReplacementArgument;
import net.minecraft.worldedit.pattern.RandomPattern;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertTrue;

class WorldEditReplacementCommandSyntaxTest {
    private static final Path SRC_MAIN_JAVA = Path.of("src/main/java");

    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void replaceUsesWorldEditReplacementArgumentInsteadOfRawStringTail() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/RegionCommands.java"));

        assertTrue(source.contains("Commands.argument(\"patterns\", WorldEditReplacementArgument.replacement())"));
        assertTrue(source.contains("WorldEditReplacementArgument.getReplacement(ctx, \"patterns\")"));
        assertFalse(source.contains("Commands.argument(\"patterns\", StringArgumentType.greedyString())"));
        assertFalse(source.contains("Commands.argument(\"from\", StringArgumentType.word())\n                .then(Commands.argument(\"to\""));
    }

    @Test
    void replaceNearUsesWorldEditReplacementArgumentInsteadOfRawStringTail() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/UtilityCommands.java"));

        assertTrue(source.contains("Commands.argument(\"from\", WorldEditMaskArgument.mask())"));
        assertTrue(source.contains("Commands.argument(\"to\", WorldEditPatternArgument.pattern())"));
        assertTrue(source.contains("WorldEditMaskArgument.getMask(ctx, \"from\")"));
        assertTrue(source.contains("WorldEditPatternArgument.getPattern(ctx, \"to\")"));
        assertFalse(source.contains("Commands.argument(\"patterns\", StringArgumentType.greedyString())"));
        assertFalse(source.contains("Commands.argument(\"patterns\", WorldEditReplacementArgument.replacementPatterns())"));
        assertFalse(source.contains("Commands.argument(\"from\", StringArgumentType.word())\n                    .then(Commands.argument(\"to\""));
    }

    @Test
    void brushCommandsUseWorldEditPatternParsingAndExpectedAliases() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/ToolCommands.java"));

        assertTrue(source.contains("registerBrushCommand(dispatcher, \"/brush\")"));
        assertTrue(source.contains("registerBrushCommand(dispatcher, \"/br\")"));
        assertTrue(source.contains("registerBrushMaskCommand(dispatcher, \"/mask\")"));
        assertTrue(source.contains("registerBrushMaskCommand(dispatcher, \"mask\")"));
        assertTrue(source.contains("Commands.argument(\"mask\", WorldEditMaskArgument.mask())"));
        assertTrue(source.contains("brushTool.setMask(mask)"));
        assertTrue(source.contains("BlockPatternParser.parse(patternText)"));
        assertTrue(source.contains("StringArgumentType.greedyString()"));
        assertTrue(source.contains("new BrushTool(\"cylinder\", parsed.pattern(), parsed.radius(), parsed.height())"));
        assertTrue(source.contains("\"smooth\""));
        assertTrue(source.contains("parsed.iterations()"));
        assertFalse(source.contains("BuiltInRegistries.BLOCK.getValue(blockId)"));
        assertFalse(source.contains("Commands.argument(\"block\", StringArgumentType.word())"));
    }

    @Test
    void brushAndToolActivationHooksAreWiredIntoServerInteractionPaths() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/server/level/ServerPlayerGameMode.java"));

        int blockBreakHook = source.indexOf("WorldEditIntegration.onBlockBreak(this.player, blockPos)");
        int vanillaDestroy = source.indexOf("block.playerWillDestroy(this.level, blockPos, blockState, this.player)");
        assertTrue(blockBreakHook >= 0);
        assertTrue(vanillaDestroy >= 0);
        assertTrue(blockBreakHook < vanillaDestroy);

        int rightClickAirHook = source.indexOf("WorldEditIntegration.onRightClickAir(serverPlayer, interactionHand)");
        int vanillaItemUse = source.indexOf("itemStack.use(level, serverPlayer, interactionHand)");
        assertTrue(rightClickAirHook >= 0);
        assertTrue(vanillaItemUse >= 0);
        assertTrue(rightClickAirHook < vanillaItemUse);

        int rightClickBlockHook = source.indexOf("WorldEditIntegration.onRightClickBlock(serverPlayer, blockPos, interactionHand)");
        int vanillaBlockUse = source.indexOf("blockState.useItemOn(");
        assertTrue(rightClickBlockHook >= 0);
        assertTrue(vanillaBlockUse >= 0);
        assertTrue(rightClickBlockHook < vanillaBlockUse);
    }

    @Test
    void brushToolSupportsClickedBlockActivationAndSeparateCylinderHeight() throws IOException {
        String source = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/tool/BrushTool.java"));

        assertTrue(source.contains("public BrushTool(String type, Pattern pattern, int radius, int secondarySize)"));
        assertTrue(source.contains("private Mask mask"));
        assertTrue(source.contains("public void setMask(Mask mask)"));
        assertTrue(source.contains("new MaskIntersection(sessionMask, mask)"));
        assertTrue(source.contains("this.brush = new CylinderBrush(secondarySize, false)"));
        assertTrue(source.contains("this.brush = new SmoothBrush(secondarySize)"));
        assertTrue(source.contains("public boolean actSecondary(ServerPlayer player, BlockPos target)"));
        assertTrue(source.contains("return applyBrush(player, target)"));
    }

    @Test
    void globalMaskCommandIsRegisteredAndStoredOnSession() throws IOException {
        String commandRoot = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/WorldEditCommands.java"));
        String generalCommands = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/command/GeneralCommands.java"));
        String localSession = Files.readString(SRC_MAIN_JAVA.resolve("net/minecraft/worldedit/session/LocalSession.java"));

        assertTrue(commandRoot.contains("GeneralCommands.register(dispatcher)"));
        assertTrue(generalCommands.contains("registerGlobalMaskCommand(dispatcher, \"/gmask\")"));
        assertTrue(generalCommands.contains("registerGlobalMaskCommand(dispatcher, \"gmask\")"));
        assertTrue(generalCommands.contains("session.setMask(mask)"));
        assertTrue(generalCommands.contains("session.setMask(null)"));
        assertTrue(localSession.contains("private Mask mask"));
        assertTrue(localSession.contains("public EditSession createEditSession(ServerLevel world)"));
        assertTrue(localSession.contains("editSession.setMask(mask)"));
    }

    @Test
    void worldEditMaskArgumentAcceptsPartialReplaceNearInputBlock() throws CommandSyntaxException {
        StringReader reader = new StringReader("sand");

        assertTrue(WorldEditMaskArgument.mask().parse(reader).test(Blocks.SAND.defaultBlockState()));
        assertFalse(reader.canRead());
    }

    @Test
    void worldEditMaskArgumentLeavesOutputPatternForReplaceNearToArgument() throws CommandSyntaxException {
        StringReader reader = new StringReader("sand 70%cobblestone,30%diorite");

        assertTrue(WorldEditMaskArgument.mask().parse(reader).test(Blocks.SAND.defaultBlockState()));
        assertTrue(reader.getRemaining().startsWith(" 70%cobblestone,30%diorite"));
    }

    @Test
    void worldEditPatternArgumentAcceptsMultipleWeightedOutputBlocks() throws CommandSyntaxException {
        StringReader reader = new StringReader("70%cobblestone,30%diorite");

        assertInstanceOf(RandomPattern.class, WorldEditPatternArgument.pattern().parse(reader));
        assertFalse(reader.canRead());
    }

    @Test
    void worldEditReplacementArgumentAcceptsMultipleInputsAndWeightedOutputs() throws CommandSyntaxException {
        StringReader reader = new StringReader("stone,dirt 70%cobblestone,30%diorite");

        WorldEditReplacementArgument.Result result = WorldEditReplacementArgument.replacementPatterns().parse(reader);

        assertFalse(reader.canRead());
        assertTrue(result.replacementPatterns().orElseThrow().from().test(Blocks.STONE.defaultBlockState()));
        assertTrue(result.replacementPatterns().orElseThrow().from().test(Blocks.DIRT.defaultBlockState()));
        assertFalse(result.replacementPatterns().orElseThrow().from().test(Blocks.GRASS_BLOCK.defaultBlockState()));
        assertInstanceOf(RandomPattern.class, result.toPattern());
    }

    @Test
    void replaceArgumentPreservesSingleOutputPatternForm() throws CommandSyntaxException {
        StringReader reader = new StringReader("cobblestone,diorite");

        WorldEditReplacementArgument.Result result = WorldEditReplacementArgument.replacement().parse(reader);

        assertFalse(reader.canRead());
        assertTrue(result.replacementPatterns().isEmpty());
        assertInstanceOf(RandomPattern.class, result.toPattern());
    }

    @Test
    void replaceNearArgumentRejectsMissingInputPattern() {
        StringReader reader = new StringReader("cobblestone,diorite");

        assertTrue(org.junit.jupiter.api.Assertions.assertThrows(
            CommandSyntaxException.class,
            () -> WorldEditReplacementArgument.replacementPatterns().parse(reader)
        ).getMessage().contains("Expected input and output block patterns"));
    }

    @Test
    void worldEditArgumentTypesAreRegisteredForCommandTreeSync() {
        assertTrue(ArgumentTypeInfos.isClassRecognized(WorldEditMaskArgument.class));
        assertTrue(ArgumentTypeInfos.isClassRecognized(WorldEditPatternArgument.class));
        assertTrue(ArgumentTypeInfos.isClassRecognized(WorldEditReplacementArgument.class));
    }

    @Test
    void replacementArgumentSyncPreservesStrictMode() {
        WorldEditReplacementArgument replacement = instantiateSynced(WorldEditReplacementArgument.replacement());
        WorldEditReplacementArgument replacementPatterns = instantiateSynced(WorldEditReplacementArgument.replacementPatterns());

        assertFalse(replacement.requiresInputAndOutputPatterns());
        assertTrue(replacementPatterns.requiresInputAndOutputPatterns());
    }

    private static WorldEditReplacementArgument instantiateSynced(WorldEditReplacementArgument argument) {
        return ArgumentTypeInfos.unpack(argument).instantiate(commandBuildContext());
    }

    private static CommandBuildContext commandBuildContext() {
        return CommandBuildContext.simple(
            RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY),
            FeatureFlags.DEFAULT_FLAGS
        );
    }
}
