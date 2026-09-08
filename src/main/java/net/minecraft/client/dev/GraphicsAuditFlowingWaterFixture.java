package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.state.BlockState;

/** Opt-in copied-world fixture. Ordinary fluid ticks create the flow; no rendering overrides. */
public final class GraphicsAuditFlowingWaterFixture {
    record Cell(BlockPos position, int kind) {
        // -1 is the stone enclosure, 0 is cleared air, 1 is the single source.
        BlockState state() {
            return (kind < 0 ? Blocks.STONE : kind == 0 ? Blocks.AIR : Blocks.WATER).defaultBlockState();
        }
    }
    private static List<Cell> installed = List.of();
    private static BlockPos origin;
    private static Direction along;
    private static String latestSurface = "null";
    private static String capturedSurface = "null";
    static List<net.minecraft.world.phys.Vec3> surfacePoints(BlockPos source, Direction direction) {
        var points = new ArrayList<net.minecraft.world.phys.Vec3>(15);
        Direction side = direction.getClockWise();
        for (int level = 2; level <= 6; level++) {
            for (double lateral : new double[] {-0.25, 0, 0.25}) {
                points.add(net.minecraft.world.phys.Vec3.atLowerCornerOf(source.relative(direction, level))
                    .add(0.5 + side.getStepX() * lateral, (8 - level) / 9.0 - 0.001,
                         0.5 + side.getStepZ() * lateral));
            }
        }
        return List.copyOf(points);
    }
    /** Read-only CPU matrix observation; never accesses GPU objects or changes a draw. */
    public static void observeWorldView(Minecraft minecraft, org.joml.Matrix4fc projection,
            org.joml.Matrix4fc view, net.minecraft.world.phys.Vec3 eye) {
        if (!requested() || origin == null) return;
        var matrix = new org.joml.Matrix4f(projection).mul(view);
        var json = new StringBuilder("[");
        for (var point : surfacePoints(origin, along)) {
            var relative = point.subtract(eye);
            var clip = matrix.transform(new org.joml.Vector4f((float)relative.x, (float)relative.y, (float)relative.z, 1));
            if (!Float.isFinite(clip.w) || clip.w <= 0) { latestSurface = "null"; return; }
            if (json.length() > 1) json.append(',');
            json.append(String.format(java.util.Locale.ROOT, "[%.6f,%.6f]",
                (clip.x / clip.w + 1) * minecraft.getWindow().getWidth() * 0.5,
                (1 - clip.y / clip.w) * minecraft.getWindow().getHeight() * 0.5));
        }
        latestSurface = json.append(']').toString();
    }
    public static void captureSurface() { capturedSurface = latestSurface; }
    public static String capturedSurface() { return capturedSurface; }
    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditFlowingWater");
    }
    static List<Cell> cells(BlockPos source, Direction direction) {
        if (direction.getAxis() == Direction.Axis.Y) throw new IllegalArgumentException("horizontal channel required");
        var plan = new ArrayList<Cell>();
        Direction side = direction.getClockWise();
        for (int x = -1; x <= 8; x++) {
            for (int z = -1; z <= 1; z++) {
                for (int y = -1; y <= 3; y++) {
                    boolean wall = y == -1 || y == 0 && (x == -1 || x == 8 || z != 0);
                    int kind = wall ? -1 : x == 0 && z == 0 && y == 0 ? 1 : 0;
                    plan.add(new Cell(source.relative(direction, x).relative(side, z).above(y), kind));
                }
            }
        }
        return List.copyOf(plan);
    }
    public static void install(Minecraft minecraft) {
        if (!requested()) return;
        if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null)
            throw new IllegalStateException("Flowing-water fixture requires a loaded copied world");
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) throw new IllegalStateException("Missing fixture server level");
        Direction forward = minecraft.player.getDirection();
        Direction direction = forward;
        BlockPos source = minecraft.player.blockPosition().relative(forward, 5).below(2);
        var plan = cells(source, direction);
        for (Cell cell : plan) {
            if (!server.isLoaded(cell.position()) || !minecraft.level.isLoaded(cell.position()))
                throw new IllegalStateException("Flowing-water fixture is not fully loaded");
        }
        // Complete the enclosure and clear the channel before introducing water.
        for (int kind : new int[] {-1, 0, 1}) {
            for (Cell cell : plan) {
                if (cell.kind() == kind) {
                    server.setBlock(cell.position(), cell.state(), 3);
                    minecraft.level.setBlock(cell.position(), cell.state(), 3);
                }
            }
        }
        origin = source;
        along = direction;
        installed = plan;
    }
    public static boolean ready(Minecraft minecraft) {
        if (!requested()) return true;
        if (installed.size() != 150 || minecraft.level == null || minecraft.getSingleplayerServer() == null) return false;
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) return false;
        for (Cell cell : installed) {
            int offset = -1;
            for (int x = 0; x < 8; x++) {
                if (cell.position().equals(origin.relative(along, x))) { offset = x; break; }
            }
            BlockState expected = offset >= 0
                ? Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, offset) : cell.state();
            if (!expected.equals(server.getBlockState(cell.position()))
                || !expected.equals(minecraft.level.getBlockState(cell.position()))) return false;
        }
        return true;
    }
    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        return "{\"fixture\":\"single-source-flow-channel-v1\",\"origin\":\""
            + (origin == null ? "" : origin.toShortString()) + "\",\"direction\":\""
            + (along == null ? "" : along.getName()) + "\",\"cells\":" + installed.size()
            + ",\"expectedLevels\":[0,1,2,3,4,5,6,7],\"complete\":" + ready(minecraft) + "}";
    }
    private GraphicsAuditFlowingWaterFixture() {}
}
