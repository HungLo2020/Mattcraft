package net.minecraft.client.dev;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

/** CPU-only observation of the copied-world overlap fixture and camera. */
public final class GraphicsAuditOverlapProjection {
    record Cell(BlockPos position, Block block) {}
    private static BlockPos target;
    private static Direction forward;
    private static String latest = "null";

    public static void configure(BlockPos position, Direction direction) {
        if (direction.getAxis() == Direction.Axis.Y) throw new IllegalArgumentException("horizontal fixture required");
        target = position.immutable();
        forward = direction;
        latest = "null";
    }

    static List<Cell> cells(BlockPos target, Direction forward) {
        var cells = new ArrayList<Cell>(42);
        Block[] layers = {Blocks.RED_STAINED_GLASS, Blocks.GREEN_STAINED_GLASS, Blocks.BLUE_STAINED_GLASS};
        for (int depth = 0; depth < 3; depth++)
            for (int lateral = -1; lateral <= 1; lateral++)
                for (int y = 0; y <= 2; y++)
                    cells.add(new Cell(target.relative(forward, depth * 2).relative(forward.getClockWise(), lateral).above(y), layers[depth]));
        for (int depth = 0; depth <= 4; depth++)
            for (int y = 0; y <= 2; y++)
                cells.add(new Cell(target.relative(forward, depth + 1).relative(forward.getClockWise(), 2).above(y), Blocks.ORANGE_STAINED_GLASS));
        return List.copyOf(cells);
    }

    static List<Vec3> points(BlockPos target, Direction forward) {
        var points = new ArrayList<Vec3>(9);
        Vec3 middle = Vec3.atCenterOf(target.relative(forward, 2));
        Direction right = forward.getClockWise();
        for (double lateral : new double[]{-0.5, 0, 0.5})
            for (double y : new double[]{0, 1, 2})
                points.add(middle.add(right.getStepX() * lateral, y, right.getStepZ() * lateral));
        return List.copyOf(points);
    }

    public static void observe(Minecraft minecraft, org.joml.Matrix4fc projection,
            org.joml.Matrix4fc view, Vec3 eye) {
        if (target == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) return;
        var server = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
        if (server == null) return;
        int matching = 0;
        for (Cell cell : cells(target, forward))
            if (minecraft.level.getBlockState(cell.position()).is(cell.block())
                    && server.getBlockState(cell.position()).is(cell.block())) matching++;
        var matrix = new org.joml.Matrix4f(projection).mul(view);
        var pixels = new StringBuilder("[");
        for (Vec3 point : points(target, forward)) {
            if (pixels.length() > 1) pixels.append(',');
            Vec3 p = point.subtract(eye);
            var clip = matrix.transform(new org.joml.Vector4f((float)p.x, (float)p.y, (float)p.z, 1));
            if (!Float.isFinite(clip.w) || clip.w <= 0) pixels.append("null");
            else pixels.append(String.format(java.util.Locale.ROOT, "[%.6f,%.6f]",
                (clip.x / clip.w + 1) * minecraft.getWindow().getWidth() * .5,
                (1 - clip.y / clip.w) * minecraft.getWindow().getHeight() * .5));
        }
        var texture = minecraft.getTextureManager().getTexture(net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS);
        int mipLevels = texture instanceof net.minecraft.client.renderer.texture.TextureAtlas atlas
            ? (atlas.mipLevel + 1) : 0;
        latest = "{\"fixture\":\"three-pane-crossing-v1\",\"origin\":[" + target.getX() + "," + target.getY() + "," + target.getZ()
            + "],\"direction\":\"" + forward.getName() + "\",\"cells\":42,\"matchingCells\":" + matching
            + ",\"complete\":" + (matching == 42) + ",\"points\":" + pixels.append(']')
            + ",\"atlasMipLevels\":" + mipLevels
            + ",\"requestedAtlasMipLevels\":" + (minecraft.options.mipmapLevels().get() + 1) + "}";
    }

    public static String receipt() { return latest; }
    private GraphicsAuditOverlapProjection() {}
}
