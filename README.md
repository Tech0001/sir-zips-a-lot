<p align="center">
  <img src="assets/brand/app-icon-crop.png" alt="Sir Zips-a-Lot, a cheerful knight in zipper armor with a folder shield" width="220" />
</p>

<h1 align="center">Sir Zips-a-Lot</h1>

<p align="center"><strong>He came. He saw. He compressed.</strong></p>

A little desktop knight with one noble quest: turn incoming order folders into individual ZIPs, without making you do the same clicks all day.

Point him at the folder where orders arrive and choose where finished ZIPs should go. He watches for new orders, waits for the files to settle, and delivers each archive to its destination. Your mouse hand may retire from active duty.

**Give him a quest**

1. On Windows, run the setup installer and open **Sir Zips-a-Lot**.
2. Choose your **Orders folder**. Each folder directly inside it is one order.
3. Choose a separate **ZIP destination**, such as `Desktop\Zipped Orders`.
4. Click **Start watching**. Add a new order folder. Let the knight handle the packing.

The ZIP takes the order's name: `Order-100` becomes `Order-100.zip`. Its files and subfolders sit directly inside the archive, so extracting into `Order-100` restores the original layout without another `Order-100` nested inside it.

**A patient knight is a good knight**

| Setting | Default | What it does |
| --- | --- | --- |
| Check for new orders | 10 seconds | Time between folder scans. Turn it up to reduce disk activity. |
| Wait for a quiet folder | 30 seconds | Time without detected changes before zipping. Use longer for slow transfers. |

Both settings are adjustable from 1 to 3,600 seconds. The quiet period is checked on each scan. Stop watching to change the settings, then start again to save them and resume.

**The knight's code**

- **One order, one ZIP.** Nested folders and empty subfolders keep their places. Original order folders stay intact.
- **Respect the old guard.** The first time you use a source and destination pair, existing order folders are left alone. Later starts pick up new arrivals, including ones that came in while the app was closed.
- **No repeat quests.** Completed orders are remembered, even after you move their ZIPs away. Give each new order a unique folder name. Existing ZIPs are never overwritten.
- **Closing time isn't quitting time.** Close the window and he keeps working in the system tray. Open him again from the tray, or right-click and choose **Quit** to finish the current archive and clock out.
- **Armor for any lighting.** Light mode, dark mode, and a remembered theme choice.

The quest log shows deliveries and any problems. His job ends at the destination folder; your usual process handles sending the ZIPs onward.

<details>
<summary><strong>At the forge — building from source</strong></summary>

Built with Rust and Tauri, with a small HTML/CSS/JavaScript interface. Windows is the main target; the app also builds locally on Linux.

With Rust, Node.js, and Tauri's platform build dependencies installed:

```sh
npm ci
npm run dev
```

Run the checks:

```sh
npm run check
npm test
```

Build the Windows installer from a Windows development machine:

```sh
npm run build -- --bundles nsis
```

The installer lands in `target/release/bundle/nsis/`.

</details>

<p align="center"><em>Small knight. Big ZIP energy.</em></p>
