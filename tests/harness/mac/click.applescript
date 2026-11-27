-- click.applescript PID: has System Events click the middle of the first
-- standard window of the process PID. System Events sends the click to the UI
-- element it finds there: enough for a button, not for the canvas of an
-- editor. mac/lib.sh uses it only when click.js could not click with the mouse.
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
