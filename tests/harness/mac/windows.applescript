-- windows.applescript PID: one "SUBROLE<tab>TITLE" line per window of the
-- process PID, and one "AXSheet<tab>TITLE" line per sheet of that window.
on run argv
	set targetPid to (item 1 of argv) as integer
	set out to ""
	tell application "System Events"
		set procs to (every application process whose unix id is targetPid)
		if procs is {} then return ""
		repeat with w in windows of (item 1 of procs)
			set out to out & my describe(w, "") & linefeed
			try
				repeat with s in sheets of w
					set out to out & my describe(s, "AXSheet") & linefeed
				end repeat
			end try
		end repeat
	end tell
	return out
end run

on describe(element, fixedRole)
	set elementRole to fixedRole
	set elementTitle to ""
	tell application "System Events"
		if elementRole is "" then
			try
				set elementRole to (subrole of element) as text
			end try
		end if
		try
			set elementTitle to (name of element) as text
		end try
	end tell
	return elementRole & tab & elementTitle
end describe
