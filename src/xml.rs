//! Recover XML with libxml2, then edit an owned tree with stable node identities.
use anyhow::{Result, anyhow};
use libxml::{
    parser::{Parser, ParserOptions},
    tree::{Document, NodeType},
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::Path,
    rc::{Rc, Weak},
};

#[derive(Clone, Debug)]
pub struct Node(Rc<RefCell<Data>>);
#[derive(Debug)]
struct Data {
    name: String,
    namespace: String,
    attrs: BTreeMap<String, String>,
    children: Vec<Node>,
    parent: Weak<RefCell<Data>>,
    kind: Kind,
}
#[derive(Clone, Debug)]
enum Kind {
    Element,
    Text(String),
    Raw(String),
}

pub struct Xml {
    pub root: Node,
    before: String,
    after: String,
}
impl Xml {
    pub fn parse(bytes: &[u8], recover: bool) -> Result<Self> {
        let doc = Parser::default().parse_string_with_options(
            bytes,
            ParserOptions {
                recover,
                no_net: true,
                ..Default::default()
            },
        )?;
        let root = doc
            .get_root_element()
            .ok_or_else(|| anyhow!("XML document has no root element."))?;
        let mut before = String::new();
        let mut after = String::new();
        let mut seen = false;
        for n in doc.as_node().get_child_nodes() {
            if n == root {
                seen = true;
                continue;
            }
            if seen {
                after.push_str(&doc.node_to_string(&n));
            } else {
                before.push_str(&doc.node_to_string(&n));
            }
        }
        Ok(Self {
            root: convert(&doc, &root),
            before,
            after,
        })
    }
    pub fn read(path: &Path, recover: bool) -> Result<Self> {
        Self::parse(&fs::read(path)?, recover).map_err(|_| {
            anyhow!(
                "Could not parse XML file: {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })
    }
    pub fn write(&self, path: &Path) -> Result<()> {
        fs::write(path, self.serialize())?;
        Ok(())
    }
    pub fn serialize(&self) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n{}{}{}",
            self.before,
            self.root.serialize(),
            self.after
        )
    }
}

fn convert(doc: &Document, source: &libxml::tree::Node) -> Node {
    if source.is_element_node() {
        let namespace = source.get_namespace();
        let prefix = namespace
            .as_ref()
            .map(|n| n.get_prefix())
            .unwrap_or_default();
        let name = if prefix.is_empty() {
            source.get_name()
        } else {
            format!("{prefix}:{}", source.get_name())
        };
        let node = Node::element(&name, &namespace.map(|n| n.get_href()).unwrap_or_default());
        for ns in source.get_namespace_declarations() {
            let prefix = ns.get_prefix();
            node.set(
                &if prefix.is_empty() {
                    "xmlns".into()
                } else {
                    format!("xmlns:{prefix}")
                },
                &ns.get_href(),
            );
        }
        for ((name, ns), value) in source.get_properties_ns() {
            let prefix = ns.map(|n| n.get_prefix()).unwrap_or_default();
            node.set(
                &if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}:{name}")
                },
                &value,
            );
        }
        for child in source.get_child_nodes() {
            node.append(convert(doc, &child));
        }
        node
    } else if source.is_text_node() || source.get_type() == Some(NodeType::CDataSectionNode) {
        Node::text(&source.get_content())
    } else {
        Node::new("", "", Kind::Raw(doc.node_to_string(source)))
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Node {}
impl Node {
    fn new(name: &str, namespace: &str, kind: Kind) -> Self {
        Self(Rc::new(RefCell::new(Data {
            name: name.into(),
            namespace: namespace.into(),
            attrs: BTreeMap::new(),
            children: Vec::new(),
            parent: Weak::new(),
            kind,
        })))
    }
    pub fn element(name: &str, namespace: &str) -> Self {
        Self::new(name, namespace, Kind::Element)
    }
    pub fn text(value: &str) -> Self {
        Self::new("", "", Kind::Text(value.into()))
    }
    pub fn like(&self, name: &str) -> Self {
        let data = self.0.borrow();
        let name = data
            .name
            .rsplit_once(':')
            .map_or_else(|| name.to_owned(), |(p, _)| format!("{p}:{name}"));
        Self::element(&name, &data.namespace)
    }
    pub fn name(&self) -> String {
        self.0
            .borrow()
            .name
            .rsplit(':')
            .next()
            .unwrap_or("")
            .to_lowercase()
    }
    pub fn namespace(&self) -> String {
        self.0.borrow().namespace.clone()
    }
    pub fn rename(&self, name: &str) {
        let replacement = self.like(name).0.borrow().name.clone();
        self.0.borrow_mut().name = replacement;
    }
    pub fn is_element(&self) -> bool {
        matches!(self.0.borrow().kind, Kind::Element)
    }
    pub fn is_text(&self) -> bool {
        matches!(self.0.borrow().kind, Kind::Text(_))
    }
    pub fn get(&self, name: &str) -> String {
        self.0.borrow().attrs.get(name).cloned().unwrap_or_default()
    }
    pub fn has(&self, name: &str) -> bool {
        self.0.borrow().attrs.contains_key(name)
    }
    pub fn set(&self, name: &str, value: &str) {
        self.0.borrow_mut().attrs.insert(name.into(), value.into());
    }
    pub fn del(&self, name: &str) {
        self.0.borrow_mut().attrs.remove(name);
    }
    pub fn attrs(&self) -> BTreeMap<String, String> {
        self.0.borrow().attrs.clone()
    }
    pub fn nodes(&self) -> Vec<Self> {
        self.0.borrow().children.clone()
    }
    pub fn children(&self) -> Vec<Self> {
        self.nodes().into_iter().filter(Self::is_element).collect()
    }
    pub fn child(&self, name: &str) -> Option<Self> {
        self.children().into_iter().find(|n| n.name() == name)
    }
    pub fn parent(&self) -> Option<Self> {
        self.0.borrow().parent.upgrade().map(Self)
    }
    pub fn descendants(&self) -> Vec<Self> {
        let mut out = Vec::new();
        for c in self.children() {
            out.push(c.clone());
            out.extend(c.descendants());
        }
        out
    }
    pub fn all(&self) -> Vec<Self> {
        let mut out = vec![self.clone()];
        out.extend(self.descendants());
        out
    }
    pub fn named(&self, name: &str) -> Vec<Self> {
        self.all()
            .into_iter()
            .filter(|n| n.name() == name)
            .collect()
    }
    pub fn detach(&self) {
        if let Some(p) = self.parent() {
            p.0.borrow_mut().children.retain(|n| n != self);
        }
        self.0.borrow_mut().parent = Weak::new();
    }
    pub fn remove_with_tail(&self) {
        if let Some(p) = self.parent() {
            let nodes = p.nodes();
            if let Some(i) = nodes.iter().position(|n| n == self) {
                for n in nodes.iter().skip(i + 1).take_while(|n| n.is_text()) {
                    n.detach();
                }
            }
        }
        self.detach();
    }
    pub fn append(&self, child: Self) {
        child.detach();
        child.0.borrow_mut().parent = Rc::downgrade(&self.0);
        self.0.borrow_mut().children.push(child);
    }
    pub fn before(&self, child: Self) {
        if let Some(p) = self.parent() {
            child.detach();
            child.0.borrow_mut().parent = Rc::downgrade(&p.0);
            let mut data = p.0.borrow_mut();
            let i = data.children.iter().position(|n| n == self).unwrap();
            data.children.insert(i, child);
        }
    }
    pub fn unwrap(&self) {
        for child in self.nodes() {
            self.before(child);
        }
        self.detach();
    }
    pub fn content(&self) -> String {
        match &self.0.borrow().kind {
            Kind::Text(s) => s.clone(),
            Kind::Raw(_) => String::new(),
            Kind::Element => self.nodes().iter().map(Self::content).collect(),
        }
    }
    pub fn normalized(&self) -> String {
        self.content()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn set_text(&self, text: &str) {
        for c in self.nodes() {
            c.detach();
        }
        self.append(Self::text(text));
    }
    pub fn classes(&self) -> Vec<String> {
        self.get("class")
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    pub fn add_class(&self, class: &str) {
        let mut c: Vec<_> = self
            .get("class")
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        if !c.iter().any(|s| s == class) {
            c.push(class.into());
        }
        self.set("class", &c.join(" "));
    }
    pub fn serialize(&self) -> String {
        let d = self.0.borrow();
        match &d.kind {
            Kind::Text(s) => escape_text(s),
            Kind::Raw(s) => s.clone(),
            Kind::Element => {
                let mut out = format!("<{}", d.name);
                for (k, v) in &d.attrs {
                    out.push_str(&format!(" {k}=\"{}\"", escape(v)));
                }
                if d.children.is_empty() {
                    out.push_str("/>");
                } else {
                    out.push('>');
                    for c in &d.children {
                        out.push_str(&c.serialize());
                    }
                    out.push_str(&format!("</{}>", d.name));
                }
                out
            }
        }
    }
}
pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub fn escape(s: &str) -> String {
    escape_text(s)
        .replace('"', "&quot;")
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#9;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaces_and_mixed_content_survive_edits() {
        let xml = Xml::parse(br#"<html xmlns="http://www.w3.org/1999/xhtml"><body>A<font>B<em>C</em>D</font>E<!--note--><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="x"/></svg></body></html>"#, false).unwrap();
        xml.root.named("font")[0].unwrap();
        assert_eq!(xml.root.normalized(), "ABCDE");
        let serialized = xml.serialize();
        let again = Xml::parse(serialized.as_bytes(), false).unwrap();
        assert_eq!(again.root.named("image")[0].get("xlink:href"), "x");
        assert!(serialized.contains("<!--note-->"));
    }
}
