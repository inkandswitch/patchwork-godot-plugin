shader_type canvas_item;

// Fill color for the background
uniform vec4 fill_color : source_color = vec4(0.0, 0.0, 0.0, 0.0);
uniform vec4 highlight_color : source_color = vec4(0.0, 0.0, 0.0, 0.0);

// Maximum number of rectangles we can handle
const int MAX_RECTANGLES = 2500;

// Array of rectangles (x, y, width, height)
uniform vec4 rectangles[MAX_RECTANGLES];

// Number of active rectangles
uniform int rectangle_count = 0;

bool point_in_rectangle(vec2 point, vec4 rect) {
    // rect format: (x, y, width, height)
    return point.x >= rect.x && 
           point.x <= rect.x + rect.z && 
           point.y >= rect.y && 
           point.y <= rect.y + rect.w;
}

void fragment() {
    // Default to the fill color
    vec4 color = fill_color;
    
    // Check if the current pixel is inside any of the rectangles
    bool inside_any_rectangle = false;
    
    for (int i = 0; i < rectangle_count; i++) {
        // Only process active rectangles
        if (i >= rectangle_count) {
            break;
        }
        
        // If the point is inside this rectangle, mark it and break
        if (point_in_rectangle(UV, rectangles[i])) {
            inside_any_rectangle = true;
            break;
        }
    }

    if (inside_any_rectangle) {
        color = highlight_color;
    }
    
    COLOR = color;
}