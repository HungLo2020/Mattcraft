package net.minecraft.worldedit.session;

import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.worldedit.core.EditSession;
import net.minecraft.worldedit.mask.BlockTypeMask;
import net.minecraft.worldedit.mask.Mask;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertSame;

class LocalSessionMaskTest {
    @BeforeAll
    static void bootstrapMinecraft() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void createEditSessionCarriesGlobalMask() {
        LocalSession session = new LocalSession();
        Mask mask = new BlockTypeMask(Blocks.STONE);

        session.setMask(mask);
        EditSession editSession = session.createEditSession(null);

        assertSame(mask, session.getMask());
        assertSame(mask, editSession.getMask());
    }
}
