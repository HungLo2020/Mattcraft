package net.minecraft.client.sounds;

import java.util.concurrent.locks.LockSupport;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.CrashReport;
import net.minecraft.client.Minecraft;
import net.minecraft.util.thread.BlockableEventLoop;

@Environment(EnvType.CLIENT)
public class SoundEngineExecutor extends BlockableEventLoop<Runnable> {
	private Thread thread = this.createThread();
	private volatile boolean shutdown;

	public SoundEngineExecutor() {
		super("Sound executor");
	}

	private Thread createThread() {
		Thread thread = new Thread(this::run);
		thread.setDaemon(true);
		thread.setName("Sound engine");
		thread.setUncaughtExceptionHandler(
			(threadx, throwable) -> Minecraft.getInstance().delayCrash(CrashReport.forThrowable(throwable, "Uncaught exception on thread: " + threadx.getName()))
		);
		thread.start();
		return thread;
	}

	public Runnable wrapRunnable(Runnable runnable) {
		return runnable;
	}

	public void schedule(Runnable runnable) {
		if (!this.shutdown) {
			super.schedule(runnable);
		}
	}

	protected boolean shouldRun(Runnable runnable) {
		return !this.shutdown;
	}

	protected Thread getRunningThread() {
		return this.thread;
	}

	private void run() {
		while (!this.shutdown) {
			this.managedBlock(() -> this.shutdown);
		}
	}

	protected void waitForTasks() {
		LockSupport.park("waiting for tasks");
	}

	public void dropQueuedTasks() {
		this.dropAllTasks();
	}

	public void shutDown() {
		this.shutdown = true;
		this.dropAllTasks();
		// Wake an idle worker without interrupting an active resource read.
		// Interrupting ZIP-backed audio I/O can close the archive channel used
		// by unrelated texture/model readers. The shutdown flag ends the loop
		// after the active task; join still waits before audio teardown.
		LockSupport.unpark(this.thread);

		try {
			this.thread.join();
		} catch (InterruptedException var2) {
			Thread.currentThread().interrupt();
		}
	}

	public void startUp() {
		this.shutdown = false;
		this.thread = this.createThread();
	}
}
