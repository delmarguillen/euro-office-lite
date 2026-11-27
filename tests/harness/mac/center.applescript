-- center.applescript PID: prints "X Y", the middle of the first standard
-- window of the process PID, in the coordinates of the screen (points, from
-- the top left corner of the main display).
on run argv
	set targetPid to (item 1 of argv) as integer
	tell application "System Events"
		set appProcess to first application process whose unix id is targetPid
		set mainWindow to first window of appProcess whose subrole is "AXStandardWindow"
		set {posX, posY} to position of mainWindow
		set {winW, winH} to size of mainWindow
	end tell
	return ((posX + winW div 2) as text) & " " & ((posY + winH div 2) as text)
end run
