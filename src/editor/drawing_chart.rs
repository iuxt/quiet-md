//! Local spreadsheet paste, following Excalidraw's charts.parse.ts.
use super::{DrawingScene, Element, ElementOptions, MAX_ELEMENTS, World, factory, scene};

pub(super) struct Spreadsheet { pub labels: Vec<String>, title: String, series: Vec<(String, Vec<f64>)> }
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Chart { Bar, Line, Radar }

fn number(s: &str) -> Option<f64> {
    let s = s.trim_end_matches('%');
    let (sign,value) = s.strip_prefix('-').map_or(("",s), |s| ("-",s));
    let value = value.trim_start_matches('+').trim_start_matches(['$', '€', '£', '¥', '₩']);
    let value = format!("{sign}{}", value.replace(',',""));
    if value.bytes().any(|c| !c.is_ascii_digit() && !b".-+".contains(&c)) { return None; }
    value.parse::<f64>().ok().filter(|v| v.is_finite())
}

const CHART_GAP: f64 = 14.;
const AXIS_LABEL_ANGLE: f64 = 5.87;

fn chart_label(text: &str, center: f64, y: f64, size: f64, family: scene::types::FontFamily, style: &ElementOptions) -> Element {
    let width = scene::text::text_width_in(text, size, family).max(1.);
    let mut element = factory::new_text(center-width/2., y, text, size, style);
    if let Element::Text(t) = &mut element {
        t.font_family = family;
        t.base.width = width;
        t.base.height = scene::text::text_height(text, size, t.line_height);
        t.text_align = scene::types::TextAlign::Center;
    }
    element
}

fn category_label(original: &str, center: f64, axis_y: f64, step: f64, style: &ElementOptions) -> Element {
    let size = 16.;
    let width = (step-8.).max(28.);
    let measure = |text: &str| scene::text::text_width(text, size);
    let mut text = original.to_owned();
    if measure(&text) > width {
        if text.split_whitespace().count()>1 && text.split_whitespace().all(|s| measure(s)<=width) {
            text = scene::text::wrap_text(&text,width,size);
        } else {
            let limit = width-measure("...");
            let mut prefix = String::new();
            for ch in original.chars() {
                prefix.push(ch);
                if measure(&prefix)>limit && prefix.len()>ch.len_utf8() { prefix.pop(); break; }
            }
            text = format!("{prefix}...");
        }
    }
    let mut element = chart_label(&text,center,axis_y,size,scene::types::FontFamily::Excalifont,style);
    if let Element::Text(t) = &mut element {
        t.base.angle = AXIS_LABEL_ANGLE;
        let rotated_height = t.base.width*AXIS_LABEL_ANGLE.sin().abs() + t.base.height*AXIS_LABEL_ANGLE.cos().abs();
        t.base.y += 2. + (rotated_height-t.base.height)/2.;
        t.original_text = original.to_owned();
        t.auto_resize = text == original;
    }
    element
}

impl Spreadsheet {
    pub(super) fn parse(text: &str) -> Option<Self> {
        let lines: Vec<_> = text.lines().filter(|s| !s.trim().is_empty()).collect();
        if lines.len()<2 { return None; }
        let mut cells = Vec::new();
        for delimiter in ['\t',',',';'] {
            let table: Vec<Vec<_>> = lines.iter().map(|s| s.split(delimiter).map(str::trim).collect()).collect();
            if table.iter().all(|r| r.len()==table[0].len()) && (table[0].len()>1 || cells.is_empty()) {
                cells = table; if cells[0].len()>1 { break; }
            }
        }
        let first = cells.first()?; let n = first.len();
        let header = if n>2 { first.iter().all(|cell| number(cell).is_none()) } else { number(first[n-1]).is_none() };
        let rows = &cells[usize::from(header)..];
        if rows.is_empty() || n<=2 && rows.len()<2 || rows.len().checked_mul(n)? > MAX_ELEMENTS/2 { return None; }
        let mut title = if header { first[if n==2 {1} else {0}].to_string() } else { String::new() };
        let (labels,series) = if n>2 && n-1 > rows.len() {
            let labels = (1..n).map(|i| if header { first[i].to_owned() } else { i.to_string() }).collect();
            let series: Vec<_> = rows.iter().map(|r| Some((r[0].to_string(),r[1..].iter().map(|s| number(s)).collect::<Option<Vec<_>>>()?))).collect::<Option<_>>()?;
            if series.len()==1 { title.clone_from(&series[0].0); }
            (labels,series)
        } else {
            let labels = rows.iter().enumerate().map(|(i,r)| if n==1 { (i+1).to_string() } else { r[0].to_string() }).collect();
            let series = (usize::from(n>1)..n).map(|i| Some((if header { first[i].to_string() } else { format!("Series {}",i.max(1)) },
                rows.iter().map(|r| number(r[i])).collect::<Option<Vec<_>>>()?))).collect::<Option<Vec<_>>>()?;
            (labels,series)
        };
        Some(Self { title, labels, series })
    }

    pub(super) fn draw(&self, kind: Chart) -> DrawingScene {
        // Strokes use the official Excalidraw palette. Each mark remains editable.
        const COLORS: [&str; 6] = ["#1971c2","#e03131","#2f9e44","#9c36b5","#f08c00","#0c8599"];
        let count = self.labels.len(); let mut doc = DrawingScene::new(); let marks = &mut doc.scene.elements;
        let style = ElementOptions { stroke_width: 1., ..Default::default() };
        let max = self.series.iter().flat_map(|(_,v)| v.iter().copied()).fold(0.,f64::max).max(1.);
        let slot = if kind==Chart::Line { 48. } else { 44.+(22.*(self.series.len()-1) as f64).min(66.) };
        let step = slot+CHART_GAP;
        let height = if kind==Chart::Line { 320. } else { 304. };
        let axis_y = height+CHART_GAP;
        let width = step*count as f64+CHART_GAP;
        let inter_bar = if self.series.len()>1 { (CHART_GAP/(self.series.len()+1) as f64).floor().max(1.) } else { 0. };
        let bar_width = ((slot-inter_bar*(self.series.len()-1) as f64)/self.series.len() as f64).max(2.);
        let cluster_start = CHART_GAP+(slot-bar_width*self.series.len() as f64-inter_bar*(self.series.len()-1) as f64)/2.;
        let min_positive = self.series.iter().flat_map(|(_,values)| values.iter().copied()).filter(|v| *v>0.).fold(f64::INFINITY,f64::min);
        let log_radar = kind==Chart::Radar && self.series.len()==1 && max/min_positive>=100.;
        let line = |pts: Vec<World>, style: &ElementOptions| factory::new_line(pts,style);
        let label = |at: World, text: &str| { let mut e = factory::new_text(at.x,at.y,text,16.,&style);
            e.base_mut().width = scene::text::text_width(text,16.).max(1.); e };
        let radar = |i: usize, r: f64| { let a = std::f64::consts::TAU*i as f64/count as f64 - std::f64::consts::FRAC_PI_2;
            World::new(152.+a.cos()*r,152.+a.sin()*r) };
        if kind==Chart::Radar {
            for (i,name) in self.labels.iter().enumerate() {
                let guide = ElementOptions { opacity: 10., roughness: 0., stroke_width: 1., ..Default::default() };
                marks.push(line(vec![radar(i,0.),radar(i,128.)],&guide));
                let at = radar(i,152.);
                marks.push(chart_label(&scene::text::wrap_text(name,140.,16.),at.x,at.y,16.,scene::types::FontFamily::Excalifont,&style));
            }
        } else {
            marks.push(line(vec![World::new(0.,axis_y),World::new(width,axis_y)],&style));
            marks.push(line(vec![World::new(0.,-CHART_GAP),World::new(0.,axis_y)],&style));
            let guide = ElementOptions { stroke_style: scene::types::StrokeStyle::Dotted, opacity: 10., stroke_width: 1., ..Default::default() };
            marks.push(line(vec![World::new(0.,0.),World::new(width,0.)],&guide));
            let maximum = format!("{max:.2}");
            for (text,y) in [(maximum.trim_end_matches('0').trim_end_matches('.'),-12.),("0",height-12.)] {
                let x = -CHART_GAP-scene::text::text_width(text,16.);
                marks.push(label(World::new(x,y),text));
            }
            for (i,name) in self.labels.iter().enumerate() {
                marks.push(category_label(name,CHART_GAP+slot/2.+step*i as f64,axis_y,step,&style));
            }
        }
        for (s,(name,values)) in self.series.iter().enumerate() {
            let color = COLORS[s%COLORS.len()];
            let ink = ElementOptions { stroke_color: color.into(), background_color: color.into(), stroke_width: 1., ..Default::default() };
            let mut pts = Vec::new();
            for (i,&value) in values.iter().enumerate() {
                let fraction = if log_radar { value.max(0.).ln_1p()/max.ln_1p() } else if kind==Chart::Line { value/max } else { (value/max).clamp(0.,1.) };
                let y = height*(1.-fraction); let x = step*i as f64;
                match kind {
                    Chart::Bar => {
                        let mut bar = factory::new_rectangle(x+cluster_start+(bar_width+inter_bar)*s as f64,y,bar_width,(height-y).max(0.01),&ink);
                        if self.series.len()>1 { bar.base_mut().fill_style = scene::types::FillStyle::Solid; }
                        else { bar.base_mut().stroke_color.clone_from(&style.stroke_color); }
                        marks.push(bar);
                    }
                    Chart::Line => {
                        let p = World::new(x+CHART_GAP+slot/2.,y); pts.push(p);
                        let radius = CHART_GAP/2.;
                        let mut dot = factory::new_ellipse(p.x-radius,p.y-radius,CHART_GAP,CHART_GAP,&ink);
                        dot.base_mut().fill_style = scene::types::FillStyle::Solid;
                        dot.base_mut().stroke_width = 2.;
                        marks.push(dot);
                    }
                    Chart::Radar => pts.push(radar(i,128.*fraction)),
                }
            }
            if pts.len()>1 {
                if kind==Chart::Radar { pts.push(pts[0]); }
                let mut trace = line(pts,&ElementOptions { stroke_width: 2., stroke_color: color.into(), ..Default::default() });
                if kind==Chart::Radar && let Element::Line(l) = &mut trace { l.polygon = true; }
                marks.push(trace);
            }
            if self.series.len()>1 { let at = World::new(0.,height+50.+s as f64*24.);
                marks.push(factory::new_rectangle(at.x,at.y,14.,14.,&ink)); marks.push(label(at+World::new(22.,-2.),name)); }
        }
        if !self.title.is_empty() {
            let center = if kind==Chart::Radar { 152. } else { width/2. };
            marks.push(chart_label(&self.title,center,-52.,36.,scene::types::FontFamily::Lilita,&style));
        }
        let group = factory::new_id(); for e in marks { e.base_mut().group_ids.push(group.clone()); }
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spreadsheet_paste_handles_wide_tables_and_numbers() {
        let data = Spreadsheet::parse("Team\tOne\tTwo\tThree\nA\t$3\t-5%\t7").unwrap();
        assert_eq!(data.labels,["One","Two","Three"]); assert_eq!(data.series[0].1,[3.,-5.,7.]);
        for kind in [Chart::Bar, Chart::Line, Chart::Radar] {
            let chart = data.draw(kind); assert!(DrawingScene::load(&chart.json().unwrap()).is_ok());
        }
        let bars = data.draw(Chart::Bar);
        assert!(bars.scene.elements.iter().any(|e| matches!(e, Element::Text(t) if t.original_text == "One" && t.base.angle == AXIS_LABEL_ANGLE)));
        assert!(Spreadsheet::parse("hello\nworld").is_none());
    }
}
