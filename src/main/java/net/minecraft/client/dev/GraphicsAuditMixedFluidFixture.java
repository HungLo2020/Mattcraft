package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

/** Opt-in copied-world fixture only; never changes renderer behavior. */
public final class GraphicsAuditMixedFluidFixture {
    private static final String VERSION = "sealed-mixed-fluid-v1";
    private static List<Cell> installed = List.of();
    private static String placement = "";
    private static BlockPos bottomWitness;
    private static String latestBottomSurface = "null";
    private static String capturedBottomSurface = "null";

    static List<net.minecraft.world.phys.Vec3> bottomSurfacePoints(BlockPos water) {
        var points = new ArrayList<net.minecraft.world.phys.Vec3>(9);
        for (double x : new double[] {0.25, 0.5, 0.75})
            for (double z : new double[] {0.25, 0.5, 0.75})
                points.add(net.minecraft.world.phys.Vec3.atLowerCornerOf(water).add(x, 0.001, z));
        return List.copyOf(points);
    }

    /** Immutable CPU camera observation only; never changes rendering state. */
    public static void observeWorldView(Minecraft minecraft, org.joml.Matrix4fc projection,
            org.joml.Matrix4fc view, net.minecraft.world.phys.Vec3 eye) {
        GraphicsAuditOverlapProjection.observe(minecraft, projection, view, eye);
        GraphicsAuditLavaFixture.observe(minecraft, projection, view, eye);
        if (bottomWitness == null) return;
        var matrix = new org.joml.Matrix4f(projection).mul(view);
        var json = new StringBuilder("[");
        for (var point : bottomSurfacePoints(bottomWitness)) {
            var relative = point.subtract(eye);
            var clip = matrix.transform(new org.joml.Vector4f((float)relative.x, (float)relative.y, (float)relative.z, 1));
            if (!Float.isFinite(clip.w) || clip.w <= 0) { latestBottomSurface = "null"; return; }
            if (json.length() > 1) json.append(',');
            json.append(String.format(java.util.Locale.ROOT, "[%.6f,%.6f]",
                (clip.x / clip.w + 1) * minecraft.getWindow().getWidth() * 0.5,
                (1 - clip.y / clip.w) * minecraft.getWindow().getHeight() * 0.5));
        }
        latestBottomSurface = json.append(']').toString();
    }
    public static void captureBottomSurface() { capturedBottomSurface = latestBottomSurface; }
    public static String capturedBottomSurface() { return capturedBottomSurface; }

    static net.minecraft.world.level.block.Block enclosureBlock() {
        return switch (System.getProperty("mattmc.dev.graphicsAuditMixedFluidEnclosure", "blue_glass")) {
            case "blue_glass", "door", "bottom_north" -> Blocks.BLUE_STAINED_GLASS;
            case "ice" -> Blocks.ICE;
            default -> throw new IllegalArgumentException("Unsupported mixed-fluid fixture enclosure");
        };
    }

    record Cell(BlockPos position, boolean water, BlockState override) {
        Cell(BlockPos position, boolean water) { this(position, water, null); }
        BlockState state() { return override != null ? override : (water ? Blocks.WATER : enclosureBlock()).defaultBlockState(); }
    }

    static List<Cell> cells(BlockPos target, Direction forward) {
        if (forward.getAxis() == Direction.Axis.Y) throw new IllegalArgumentException("horizontal fixture direction required");
        boolean bottomNorth = "bottom_north".equals(System.getProperty("mattmc.dev.graphicsAuditMixedFluidEnclosure"));
        if (bottomNorth && forward != Direction.WEST) throw new IllegalArgumentException("bottom-north fixture requires west orientation");
        var cells = new ArrayList<Cell>(64);
        Direction right = forward.getClockWise();
        for (int depth = -1; depth <= 2; depth++) {
            for (int lateral = -1; lateral <= 2; lateral++) {
                for (int height = -1; height <= 2; height++) {
                    boolean interior = depth >= 0 && depth <= 1 && lateral >= 0 && lateral <= 1 && height >= 0 && height <= 1;
                    boolean originalGlass = depth == 0 && lateral == 0 || depth == 1 && lateral == 1 && height == 0;
                    if (bottomNorth && depth == 0 && lateral == 2 && height == 0) {
                        cells.add(new Cell(target.relative(right, 2), false, Blocks.STONE.defaultBlockState()));
                        continue;
                    }
                    if ("door".equals(System.getProperty("mattmc.dev.graphicsAuditMixedFluidEnclosure"))
                        && depth == -1 && lateral == 1 && (height == 0 || height == 1)) {
                        var door = Blocks.OAK_DOOR.defaultBlockState()
                            .setValue(net.minecraft.world.level.block.DoorBlock.FACING, forward.getOpposite())
                            .setValue(net.minecraft.world.level.block.DoorBlock.HALF, height == 0
                                ? net.minecraft.world.level.block.state.properties.DoubleBlockHalf.LOWER
                                : net.minecraft.world.level.block.state.properties.DoubleBlockHalf.UPPER);
                        cells.add(new Cell(target.relative(forward, depth).relative(right, lateral).above(height), false, door));
                        continue;
                    }
                    cells.add(new Cell(target.relative(forward, depth).relative(right, lateral).above(height),
                        interior && !originalGlass));
                }
            }
        }
        return List.copyOf(cells);
    }

    public static boolean install(Minecraft minecraft, BlockPos target, Direction forward) {
        if (minecraft.level == null || minecraft.getSingleplayerServer() == null) return false;
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) return false;
        List<Cell> plan = cells(target, forward);
        // Preflight the entire fixture before any mutation. Build the sealed glass
        // shell first, so ordinary fluid ticks cannot create an FPS-dependent waterfall.
        for (Cell cell : plan) {
            if (!server.isLoaded(cell.position()) || !minecraft.level.isLoaded(cell.position())) return false;
        }
        // Enclosure/support first, then the ordinary two-block door, then
        // water. Neighbor updates and fluid ticks remain normal throughout.
        for (int stage = 0; stage < 3; stage++) {
            for (Cell cell : plan) {
                if ((cell.water() ? 2 : cell.override() != null ? 1 : 0) == stage) {
                    server.setBlock(cell.position(), cell.state(), 3);
                    minecraft.level.setBlock(cell.position(), cell.state(), 3);
                }
            }
        }
        installed = plan;
        bottomWitness = "bottom_north".equals(System.getProperty("mattmc.dev.graphicsAuditMixedFluidEnclosure"))
            ? target.north() : null;
        placement = target.getX() + "," + target.getY() + "," + target.getZ() + "/" + forward.getName();
        return true;
    }

    public static String receipt(Minecraft minecraft) {
        int matching = 0;
        if (minecraft.level != null && minecraft.getSingleplayerServer() != null) {
            var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
            if (server != null) {
                for (Cell cell : installed) {
                    if (minecraft.level.getBlockState(cell.position()).equals(cell.state())
                        && server.getBlockState(cell.position()).equals(cell.state())) matching++;
                }
            }
        }
        return "{\"fixture\":\"" + VERSION + "\",\"placement\":\"" + placement
            + "\",\"variant\":\"" + System.getProperty("mattmc.dev.graphicsAuditMixedFluidEnclosure", "blue_glass")
            + "\",\"enclosure\":\"" + enclosureBlock().builtInRegistryHolder().key().location()
            + "\",\"cells\":" + installed.size() + ",\"matchingCells\":" + matching
            + ",\"complete\":" + (installed.size() == 64 && matching == 64) + "}";
    }

    private GraphicsAuditMixedFluidFixture() {}
}
