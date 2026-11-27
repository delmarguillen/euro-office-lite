// click.js X Y: a left click of the mouse at the point X Y of the screen, in
// the coordinates center.applescript prints. Run with osascript -l JavaScript.
//
// The pointer moves there, then the button goes down and up, as events of the
// mouse itself (Quartz events posted at the HID tap), like xdotool on Linux
// and mouse_event on Windows. The "click at" of System Events is not that: it
// looks for the UI element at the point and sends it a click, which presses a
// button of a page but does nothing on the canvas of an editor.
//
// osascript needs the Accessibility permission to post the events.
ObjC.import('CoreGraphics');

// CGEventType
var MOUSE_MOVED = 5;
var LEFT_MOUSE_DOWN = 1;
var LEFT_MOUSE_UP = 2;
// kCGMouseButtonLeft
var LEFT_BUTTON = 0;
// kCGHIDEventTap
var HID_TAP = 0;

function post(type, point) {
	var event = $.CGEventCreateMouseEvent(null, type, point, LEFT_BUTTON);
	$.CGEventPost(HID_TAP, event);
}

function run(argv) {
	var point = { x: Number(argv[0]), y: Number(argv[1]) };
	if (argv.length !== 2 || isNaN(point.x) || isNaN(point.y)) {
		throw new Error('usage: click.js X Y');
	}
	post(MOUSE_MOVED, point);
	delay(0.2);
	post(LEFT_MOUSE_DOWN, point);
	delay(0.1);
	post(LEFT_MOUSE_UP, point);
	return 'clicked at ' + point.x + ' ' + point.y;
}
