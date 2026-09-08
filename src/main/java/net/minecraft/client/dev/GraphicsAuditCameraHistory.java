package net.minecraft.client.dev;

import net.minecraft.client.player.LocalPlayer;

/** Shared deterministic fixture state; never called by normal gameplay. */
public final class GraphicsAuditCameraHistory {
    public static void settle(LocalPlayer player) {
        player.xBob = player.xBobO = player.getXRot();
        player.yBob = player.yBobO = player.getYRot();
    }

    public static String receipt(LocalPlayer player) {
        if (player == null) return "null";
        return String.format(java.util.Locale.ROOT, "[%.6f,%.6f,%.6f,%.6f]",
            player.xBobO, player.xBob, player.yBobO, player.yBob);
    }

    private GraphicsAuditCameraHistory() {}
}
