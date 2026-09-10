package net.minecraft.client.dev;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
class GraphicsAuditGuiFoilAtlasReuseTest {
    @Test void movingPhaseRequiresEightUniqueSlotsLinkedToCurrentFrameDraws() {
        String trace="mattmc.dev.guiItemRasterTrace", phase="mattmc.dev.graphicsAuditGuiItemFoilPhase";
        String oldTrace=System.getProperty(trace),oldPhase=System.getProperty(phase);
        try {
            System.setProperty(trace,"true");System.setProperty(phase,"10000");
            GraphicsAuditGuiFoilTiming.resetForTest();
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.beginItem(145,221);
            GraphicsAuditGuiFoilTiming.observeScaledTicks(10000,8);
            GraphicsAuditGuiFoilTiming.endItem();
            GraphicsAuditGuiFoilTiming.observeAtlasWrite(145,221,0,0);
            assertFalse(GraphicsAuditGuiFoilTiming.readyForCapture());
            for(int i=1;i<8;i++) GraphicsAuditGuiFoilTiming.observeAtlasReuse(145+20*i,221,0,0);
            assertTrue(GraphicsAuditGuiFoilTiming.readyForCapture());
            System.setProperty(phase,"40000");
            assertFalse(GraphicsAuditGuiFoilTiming.readyForCapture());
            System.setProperty(phase,"10000");
            GraphicsAuditGuiFoilTiming.observeAtlasReuse(165,221,0,0);
            assertFalse(GraphicsAuditGuiFoilTiming.readyForCapture(),"duplicate targets are not evidence");
            GraphicsAuditGuiFoilTiming.beginFrame();
            for(int i=1;i<8;i++) GraphicsAuditGuiFoilTiming.observeAtlasReuse(145+20*i,221,0,0);
            assertFalse(GraphicsAuditGuiFoilTiming.readyForCapture(),"previous-frame writes cannot qualify");
        } finally {
            GraphicsAuditGuiFoilTiming.resetForTest();
            if(oldTrace==null)System.clearProperty(trace);else System.setProperty(trace,oldTrace);
            if(oldPhase==null)System.clearProperty(phase);else System.setProperty(phase,oldPhase);
        }
    }
    @Test void aliasesReferenceOnlyActualTimedWritesInTheCurrentFrame() {
        String key="mattmc.dev.guiItemRasterTrace", old=System.getProperty(key);
        try {
            System.setProperty(key,"true");
            GraphicsAuditGuiFoilTiming.resetForTest();
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.beginItem(145,221);
            GraphicsAuditGuiFoilTiming.observeScaledTicks(0,8);
            GraphicsAuditGuiFoilTiming.endItem();
            GraphicsAuditGuiFoilTiming.observeAtlasWrite(145,221,48,0);
            GraphicsAuditGuiFoilTiming.observeAtlasReuse(185,221,48,0);
            String snapshot=GraphicsAuditGuiFoilTiming.snapshot();
            assertTrue(snapshot.contains("\"sourceX\":145,\"sourceY\":221"));
            assertTrue(snapshot.contains("\"atlasX\":48,\"atlasY\":0"));
            GraphicsAuditGuiFoilTiming.beginFrame();
            GraphicsAuditGuiFoilTiming.observeAtlasReuse(185,221,48,0);
            assertTrue(GraphicsAuditGuiFoilTiming.snapshot().contains("\"atlasAliases\":[]"));
            assertTrue(snapshot.contains("\"sourceX\":145"),"previous capture snapshots must remain immutable");
        } finally {
            GraphicsAuditGuiFoilTiming.resetForTest();
            if(old==null) System.clearProperty(key); else System.setProperty(key,old);
        }
    }
}
