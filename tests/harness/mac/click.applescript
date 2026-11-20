-- click.applescript PID: clicks the middle of the first standard window of
-- the process PID.
on run argv
	set targetPid to (item 1 of argv) as integer
	tell application "System Events"
		set appProcess to first application process whose unix id is targetPid
		set mainWindow to first window of appProcess whose subrole is "AXStandardWindow"
		set {posX, posY} to position of mainWindow
		set {winW, winH} to size of mainWindow
		tell appProcess to click at {posX + winW div 2, posY + winH div 2}
	end tell
end run
