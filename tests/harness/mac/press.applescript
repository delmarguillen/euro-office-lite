-- press.applescript PID LABEL: clicks the middle of the first element of the
-- standard window of PID whose name, description or value is LABEL (case
-- matters). The search goes level by level, so the shallowest element wins:
-- an HTML button before the text inside it. Fails when there is none.
on run argv
	set targetPid to (item 1 of argv) as integer
	set label to item 2 of argv
	tell application "System Events"
		set appProcess to first application process whose unix id is targetPid
		set mainWindow to first window of appProcess whose subrole is "AXStandardWindow"
	end tell
	set found to my findLabelled(mainWindow, label)
	if found is missing value then error "nothing named " & label & " in the window" number 1
	tell application "System Events"
		set {posX, posY} to position of found
		set {elementW, elementH} to size of found
		tell appProcess to click at {posX + elementW div 2, posY + elementH div 2}
	end tell
	return "pressed " & label
end run

-- A web view has thousands of elements: the budget keeps a search for
-- something that is not there to a few seconds.
on findLabelled(root, label)
	set budget to 3000
	set level to {root}
	repeat while level is not {}
		set nextLevel to {}
		repeat with i from 1 to count of level
			set children to my childrenOf(item i of level)
			repeat with j from 1 to count of children
				set child to item j of children
				if my isLabelled(child, label) then return child
				set budget to budget - 1
				if budget is 0 then return missing value
				set end of nextLevel to child
			end repeat
		end repeat
		set level to nextLevel
	end repeat
	return missing value
end findLabelled

on childrenOf(element)
	tell application "System Events"
		try
			return UI elements of element
		end try
	end tell
	return {}
end childrenOf

on isLabelled(element, label)
	set labels to my labelsOf(element)
	considering case
		return labels contains {label}
	end considering
end isLabelled

-- The name, description and value of an element, as text, from one Apple
-- event. A field the element lacks is left out.
on labelsOf(element)
	set found to {}
	tell application "System Events"
		try
			set props to properties of element
		on error
			return my labelsOneByOne(element)
		end try
		try
			set end of found to (name of props) as text
		end try
		try
			set end of found to (description of props) as text
		end try
		try
			set end of found to (value of props) as text
		end try
	end tell
	return found
end labelsOf

-- For an element whose properties cannot be read at once: an Apple event per field.
on labelsOneByOne(element)
	set found to {}
	tell application "System Events"
		try
			set end of found to (name of element) as text
		end try
		try
			set end of found to (description of element) as text
		end try
		try
			set end of found to (value of element) as text
		end try
	end tell
	return found
end labelsOneByOne
