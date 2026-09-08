package net.minecraft.client.sounds;

import java.nio.file.FileSystems;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.locks.LockSupport;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import static org.junit.jupiter.api.Assertions.*;

class SoundEngineExecutorTest {
    @TempDir Path temporary;

    @Test void shutdownFinishesActiveZipReadWithoutClosingResourcesForOtherConsumers() throws Exception {
        Path archive = temporary.resolve("resources.zip");
        byte[] content = new byte[32768];
        for (int i = 0; i < content.length; i++) content[i] = (byte)(i * 31);
        try (var zip = new ZipOutputStream(Files.newOutputStream(archive))) {
            zip.putNextEntry(new ZipEntry("audio.ogg"));
            zip.write(content);
            zip.closeEntry();
        }
        try (var zip = FileSystems.newFileSystem(archive)) {
            var executor = new SoundEngineExecutor();
            var entered = new CountDownLatch(1);
            var release = new AtomicBoolean();
            var failure = new AtomicReference<Throwable>();
            var queuedRan = new AtomicBoolean();
            Thread stopping = new Thread(executor::shutDown);
            try {
                executor.schedule(() -> {
                    entered.countDown();
                    // Preserve the interrupt flag, as a resource read would.
                    while (!release.get()) LockSupport.parkNanos(100_000);
                    try {
                        assertArrayEquals(content, Files.readAllBytes(zip.getPath("/audio.ogg")));
                    } catch (Throwable error) {
                        failure.set(error);
                    }
                });
                assertTrue(entered.await(5, TimeUnit.SECONDS));
                executor.schedule(() -> queuedRan.set(true));
                stopping.start();
                long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
                while (stopping.getState() != Thread.State.WAITING && System.nanoTime() < deadline)
                    LockSupport.parkNanos(100_000);
                assertEquals(Thread.State.WAITING, stopping.getState(), "shutdown must join the active task");
                release.set(true);
                stopping.join(5000);
                assertFalse(stopping.isAlive());
                assertNull(failure.get(), "shutdown interrupted the shared ZIP resource read");
                assertFalse(queuedRan.get(), "shutdown must discard queued audio work");
                assertArrayEquals(content, Files.readAllBytes(zip.getPath("/audio.ogg")),
                    "other resource consumers must retain a usable archive");
                executor.startUp();
                var restarted = new CountDownLatch(1);
                executor.schedule(restarted::countDown);
                assertTrue(restarted.await(5, TimeUnit.SECONDS));
            } finally {
                release.set(true);
                if (stopping.getState() != Thread.State.NEW) stopping.join(5000);
                executor.shutDown();
            }
        }
    }

    @Test void shutdownWakesIdleWorker() throws Exception {
        var executor = new SoundEngineExecutor();
        Thread stopping = new Thread(executor::shutDown);
        stopping.start();
        stopping.join(5000);
        assertFalse(stopping.isAlive());
    }
}
