package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

/** Opt-in copied-world lava fixture; normal fluid and animation ticks remain untouched. */
public final class GraphicsAuditLavaFixture {
    record Cell(BlockPos position, int kind) {
        BlockState state() {
            return switch (kind) {
                case -1 -> Blocks.STONE.defaultBlockState();
                case 0 -> Blocks.AIR.defaultBlockState();
                case 1 -> Blocks.GLASS.defaultBlockState();
                case 2 -> Blocks.LAVA.defaultBlockState();
                default -> throw new IllegalArgumentException("Unknown fixture cell");
            };
        }
    }
    private static List<Cell> installed = List.of();
    private static BlockPos origin;
    private static Direction along;
    private static String latestSurface = "null";
    private static String capturedSurface = "null";
    private static GraphicsAuditParticleOcclusion particles;
    private static String capturedParticles = "null";
    public static void beginParticles() { if (requested() && particles != null) particles.begin(); }
    public static void endParticles() { if (requested() && particles != null) particles.end(); }
    public static void unknownParticleGroup() { if (requested() && particles != null) particles.unknownGroup(); }
    public static void observeParticle(float x, float y, float z, float qx, float qy, float qz, float qw,
            float size, int color) {
        if (requested() && particles != null) particles.quad(x, y, z, qx, qy, qz, qw, size, color);
    }
    public static boolean samplesUnobstructed() { return !requested() || particles != null && particles.ready(); }
    public static String capturedParticles() { return capturedParticles; }
    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditLavaCycleCapture");
    }
    public static boolean flowing() {
        return switch (System.getProperty("mattmc.dev.graphicsAuditLavaSprite", "still")) {
            case "still" -> false;
            case "flow" -> true;
            default -> throw new IllegalArgumentException("Unsupported diagnostic lava sprite");
        };
    }
    static List<Cell> flowingCells(BlockPos source, Direction direction) {
        if (direction.getAxis() == Direction.Axis.Y) throw new IllegalArgumentException("Horizontal lava channel required");
        var cells = new ArrayList<Cell>();
        for (int x = -1; x <= 4; x++)
            for (int z = -1; z <= 1; z++)
                for (int y = -1; y <= 3; y++) {
                    boolean wall = y == -1 || y == 0 && (x == -1 || x == 4 || z != 0);
                    cells.add(new Cell(source.relative(direction, x).relative(direction.getClockWise(), z).above(y),
                        wall ? -1 : x == 0 && z == 0 && y == 0 ? 2 : 0));
                }
        return List.copyOf(cells);
    }
    static List<Vec3> flowingSurfacePoints(BlockPos source, Direction direction) {
        if (direction.getAxis() == Direction.Axis.Y) throw new IllegalArgumentException("Horizontal lava channel required");
        var points = new ArrayList<Vec3>();
        var side = direction.getClockWise();
        // Overworld levels 2,4,6 give this middle cell corner heights 5/9
        // upstream and 3/9 downstream. No source-height weighting at this cell.
        for (double forward : new double[] {-0.25, 0, 0.25})
            for (double lateral : new double[] {-0.25, 0, 0.25})
                points.add(Vec3.atLowerCornerOf(source.relative(direction, 2)).add(
                    0.5 + direction.getStepX() * forward + side.getStepX() * lateral,
                    (4 - 2 * forward) / 9.0 - 0.001,
                    0.5 + direction.getStepZ() * forward + side.getStepZ() * lateral));
        return List.copyOf(points);
    }
    static List<Cell> cells(BlockPos source) {
        var cells = new ArrayList<Cell>();
        for (int y = -1; y <= 1; y++)
            for (int x = -2; x <= 2; x++)
                for (int z = -2; z <= 2; z++)
                    cells.add(new Cell(source.offset(x, y, z),
                        y < 0 ? -1 : y > 0 ? 0 : Math.abs(x) <= 1 && Math.abs(z) <= 1 ? 2 : 1));
        return List.copyOf(cells);
    }
    static List<Vec3> surfacePoints(BlockPos source) {
        var points = new ArrayList<Vec3>();
        for (double x : new double[] {0.25, 0.5, 0.75})
            for (double z : new double[] {0.25, 0.5, 0.75})
                points.add(Vec3.atLowerCornerOf(source).add(x, 8.0 / 9.0 - 0.001, z));
        return List.copyOf(points);
    }
    public static void install(Minecraft minecraft) {
        if (!requested()) return;
        if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null)
            throw new IllegalStateException("Lava fixture requires a loaded copied world");
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) throw new IllegalStateException("Missing fixture server level");
        Direction direction = minecraft.player.getDirection();
        var source = minecraft.player.blockPosition().relative(direction, 5).below(flowing() ? 2 : 1);
        var plan = flowing() ? flowingCells(source, direction) : cells(source);
        for (var cell : plan)
            if (!server.isLoaded(cell.position()) || !minecraft.level.isLoaded(cell.position()))
                throw new IllegalStateException("Lava fixture is not fully loaded");
        // Complete support, clearance and containment before introducing the source.
        for (int kind : new int[] {-1, 0, 1, 2})
            for (var cell : plan)
                if (cell.kind() == kind) {
                    server.setBlock(cell.position(), cell.state(), 3);
                    minecraft.level.setBlock(cell.position(), cell.state(), 3);
                }
        origin = source;
        along = direction;
        installed = plan;
    }
    private static int matchingCells(Minecraft minecraft) {
        if (minecraft.level == null || minecraft.getSingleplayerServer() == null) return 0;
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) return 0;
        int matching = 0;
        for (var cell : installed) {
            var expected = cell.state();
            if (flowing())
                for (int offset = 0; offset < 4; offset++)
                    if (cell.position().equals(origin.relative(along, offset)))
                        expected = Blocks.LAVA.defaultBlockState().setValue(LiquidBlock.LEVEL, offset * 2);
            if (expected.equals(server.getBlockState(cell.position()))
                && expected.equals(minecraft.level.getBlockState(cell.position()))) matching++;
        }
        return matching;
    }
    public static boolean ready(Minecraft minecraft) {
        int count = flowing() ? 90 : 75;
        return !requested() || installed.size() == count && matchingCells(minecraft) == count;
    }
    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        return "{\"fixture\":\"" + (flowing() ? "single-source-lava-flow-v1" : "sealed-lava-basin-v2") + "\",\"origin\":\""
            + (origin == null ? "" : origin.toShortString()) + "\",\"cells\":" + installed.size()
            + (flowing() ? ",\"expectedLevels\":[0,2,4,6],\"direction\":\"" + (along == null ? "" : along.getName()) + "\"" : "")
            + ",\"matchingCells\":" + matchingCells(minecraft) + ",\"complete\":" + ready(minecraft) + "}";
    }
    /** CPU projection observation only; never reads or changes GPU state. */
    public static void observe(Minecraft minecraft, org.joml.Matrix4fc projection,
            org.joml.Matrix4fc view, Vec3 eye) {
        if (!requested() || origin == null) return;
        var matrix = new org.joml.Matrix4f(projection).mul(view);
        var points = flowing() ? flowingSurfacePoints(origin, along) : surfacePoints(origin);
        particles = new GraphicsAuditParticleOcclusion(matrix, minecraft.getWindow().getWidth(),
            minecraft.getWindow().getHeight(), points, eye);
        var json = new StringBuilder("[");
        for (var point : flowing() ? flowingSurfacePoints(origin, along) : surfacePoints(origin)) {
            var relative = point.subtract(eye);
            var clip = matrix.transform(new org.joml.Vector4f((float)relative.x, (float)relative.y, (float)relative.z, 1));
            if (!Float.isFinite(clip.w) || clip.w <= 0) { latestSurface = "null"; return; }
            if (json.length() > 1) json.append(',');
            json.append(String.format(Locale.ROOT, "[%.6f,%.6f]",
                (clip.x / clip.w + 1) * minecraft.getWindow().getWidth() * 0.5,
                (1 - clip.y / clip.w) * minecraft.getWindow().getHeight() * 0.5));
        }
        latestSurface = json.append(']').toString();
    }
    public static void captureSurface() {
        capturedSurface = latestSurface;
        capturedParticles = particles == null ? "null" : particles.receipt();
    }
    public static String capturedSurface() { return capturedSurface; }
    private GraphicsAuditLavaFixture() {}
}
