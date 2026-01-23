use crate::faults::FaultInjector;
use tracing::{debug, info};

/// NNTP command parsed from client input
#[derive(Debug, Clone)]
pub enum Command {
    Capabilities,
    AuthInfoUser(String),
    AuthInfoPass(String),
    Group(String),
    Article(String),
    Head(String),
    Body(String),
    Stat(String),
    Xover(String),
    Xhdr(String, String),
    ListGroup(Option<String>),
    List(Option<String>),
    XfeatureCompress,
    Quit,
    Unknown(String),
}

impl Command {
    pub fn parse(line: &str) -> Self {
        let line = line.trim();
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let cmd = parts[0].to_uppercase();
        let arg = parts.get(1).map(|s| s.to_string()).unwrap_or_default();

        match cmd.as_str() {
            "CAPABILITIES" => Command::Capabilities,
            "AUTHINFO" => {
                let auth_parts: Vec<&str> = arg.splitn(2, ' ').collect();
                match auth_parts[0].to_uppercase().as_str() {
                    "USER" => Command::AuthInfoUser(auth_parts.get(1).unwrap_or(&"").to_string()),
                    "PASS" => Command::AuthInfoPass(auth_parts.get(1).unwrap_or(&"").to_string()),
                    _ => Command::Unknown(line.to_string()),
                }
            }
            "GROUP" => Command::Group(arg),
            "ARTICLE" => Command::Article(arg),
            "HEAD" => Command::Head(arg),
            "BODY" => Command::Body(arg),
            "STAT" => Command::Stat(arg),
            "XOVER" | "OVER" => Command::Xover(arg),
            "XHDR" | "HDR" => {
                let hdr_parts: Vec<&str> = arg.splitn(2, ' ').collect();
                Command::Xhdr(
                    hdr_parts[0].to_string(),
                    hdr_parts.get(1).unwrap_or(&"").to_string(),
                )
            }
            "LISTGROUP" => Command::ListGroup(if arg.is_empty() { None } else { Some(arg) }),
            "LIST" => Command::List(if arg.is_empty() { None } else { Some(arg) }),
            "XFEATURE" if arg.to_uppercase().contains("COMPRESS") => Command::XfeatureCompress,
            "QUIT" => Command::Quit,
            _ => Command::Unknown(line.to_string()),
        }
    }
}

/// NNTP response generator with fault injection
pub struct ResponseGenerator {
    injector: FaultInjector,
    current_group: Option<String>,
    authenticated: bool,
    compression_enabled: bool,
}

impl ResponseGenerator {
    pub fn new(injector: FaultInjector) -> Self {
        Self {
            injector,
            current_group: None,
            authenticated: false,
            compression_enabled: false,
        }
    }

    /// Generate greeting response
    pub fn greeting(&self, message: &str) -> Vec<u8> {
        let response = format!("200 {}\r\n", message);
        self.injector.apply_status_faults(&response)
    }

    /// Process a command and generate response
    pub fn process(&mut self, cmd: Command) -> Response {
        debug!(?cmd, "Processing command");

        match cmd {
            Command::Capabilities => self.capabilities(),
            Command::AuthInfoUser(user) => self.auth_user(&user),
            Command::AuthInfoPass(pass) => self.auth_pass(&pass),
            Command::Group(name) => self.group(&name),
            Command::Article(id) => self.article(&id),
            Command::Head(id) => self.head(&id),
            Command::Body(id) => self.body(&id),
            Command::Stat(id) => self.stat(&id),
            Command::Xover(range) => self.xover(&range),
            Command::Xhdr(header, range) => self.xhdr(&header, &range),
            Command::ListGroup(group) => self.listgroup(group.as_deref()),
            Command::List(what) => self.list(what.as_deref()),
            Command::XfeatureCompress => self.xfeature_compress(),
            Command::Quit => self.quit(),
            Command::Unknown(line) => self.unknown(&line),
        }
    }

    fn capabilities(&self) -> Response {
        let lines = vec![
            "VERSION 2".to_string(),
            "READER".to_string(),
            "POST".to_string(),
            "OVER".to_string(),
            "HDR".to_string(),
            "XFEATURE-COMPRESS GZIP".to_string(),
            "AUTHINFO USER".to_string(),
        ];
        Response::multiline(101, "Capability list:", lines, &self.injector)
    }

    fn auth_user(&mut self, _user: &str) -> Response {
        info!("Auth user received");
        Response::single(381, "Password required", &self.injector)
    }

    fn auth_pass(&mut self, _pass: &str) -> Response {
        self.authenticated = true;
        info!("Authentication successful");
        Response::single(281, "Authentication accepted", &self.injector)
    }

    fn group(&mut self, name: &str) -> Response {
        self.current_group = Some(name.to_string());
        // Mock group info: count low high name
        Response::single(211, &format!("1000 1 1000 {}", name), &self.injector)
    }

    fn article(&self, id: &str) -> Response {
        let (headers, body) = self.mock_article(id);
        let mut all_lines = headers;
        all_lines.push(String::new()); // Blank line between headers and body
        all_lines.extend(body);

        Response::multiline(220, &format!("0 {} article", id), all_lines, &self.injector)
    }

    fn head(&self, id: &str) -> Response {
        let (headers, _) = self.mock_article(id);
        Response::multiline(221, &format!("0 {} head", id), headers, &self.injector)
    }

    fn body(&self, id: &str) -> Response {
        let (_, body) = self.mock_article(id);
        Response::multiline(222, &format!("0 {} body", id), body, &self.injector)
    }

    fn stat(&self, id: &str) -> Response {
        Response::single(223, &format!("0 {}", id), &self.injector)
    }

    fn xover(&self, range: &str) -> Response {
        // Parse range (e.g., "1-10" or "1")
        let (start, end) = Self::parse_range(range);
        let mut lines = Vec::new();

        for i in start..=end.min(start + 99) {
            // Limit to 100 entries
            lines.push(format!(
                "{}\tTest Subject {}\tposter@example.com\tSat, 01 Jan 2024 12:00:00 +0000\t<msg{}@example.com>\t\t1234\t10",
                i, i, i
            ));
        }

        Response::multiline(224, "Overview information follows", lines, &self.injector)
    }

    fn xhdr(&self, header: &str, range: &str) -> Response {
        let (start, end) = Self::parse_range(range);
        let mut lines = Vec::new();

        for i in start..=end.min(start + 99) {
            let value = match header.to_lowercase().as_str() {
                "subject" => format!("Test Subject {}", i),
                "from" => "poster@example.com".to_string(),
                "date" => "Sat, 01 Jan 2024 12:00:00 +0000".to_string(),
                "message-id" => format!("<msg{}@example.com>", i),
                _ => format!("header-value-{}", i),
            };
            lines.push(format!("{} {}", i, value));
        }

        Response::multiline(221, &format!("{} header follows", header), lines, &self.injector)
    }

    fn listgroup(&self, group: Option<&str>) -> Response {
        let group_name = group
            .or(self.current_group.as_deref())
            .unwrap_or("misc.test");

        let lines: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
        Response::multiline(
            211,
            &format!("100 1 100 {} list follows", group_name),
            lines,
            &self.injector,
        )
    }

    fn list(&self, what: Option<&str>) -> Response {
        match what {
            Some("ACTIVE") | None => {
                let lines = vec![
                    "misc.test 1000 1 y".to_string(),
                    "alt.binaries.test 5000 1 y".to_string(),
                ];
                Response::multiline(215, "list of newsgroups follows", lines, &self.injector)
            }
            Some("OVERVIEW.FMT") => {
                let lines = vec![
                    "Subject:".to_string(),
                    "From:".to_string(),
                    "Date:".to_string(),
                    "Message-ID:".to_string(),
                    "References:".to_string(),
                    ":bytes".to_string(),
                    ":lines".to_string(),
                ];
                Response::multiline(215, "Order of fields in overview database", lines, &self.injector)
            }
            _ => Response::single(501, "Syntax error", &self.injector),
        }
    }

    fn xfeature_compress(&mut self) -> Response {
        self.compression_enabled = true;
        info!("Compression enabled");
        Response::single(290, "XFEATURE COMPRESS GZIP enabled", &self.injector)
    }

    fn quit(&self) -> Response {
        Response::single(205, "Closing connection", &self.injector)
    }

    fn unknown(&self, line: &str) -> Response {
        info!(command = line, "Unknown command");
        Response::single(500, "Unknown command", &self.injector)
    }

    fn parse_range(range: &str) -> (u64, u64) {
        if range.contains('-') {
            let parts: Vec<&str> = range.split('-').collect();
            let start = parts[0].parse().unwrap_or(1);
            let end = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(start);
            (start, end)
        } else {
            let num = range.parse().unwrap_or(1);
            (num, num)
        }
    }

    fn mock_article(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let mut headers = vec![
            format!("Message-ID: {}", if id.starts_with('<') { id.to_string() } else { format!("<{}>", id) }),
            "From: poster@example.com".to_string(),
            "Subject: Test Article".to_string(),
            "Date: Sat, 01 Jan 2024 12:00:00 +0000".to_string(),
            "Newsgroups: misc.test".to_string(),
            "Path: fault-nntp-server!not-for-mail".to_string(),
        ];

        let mut body = vec![
            "This is a test article body.".to_string(),
            "It has multiple lines.".to_string(),
            "".to_string(),
            "And a blank line above.".to_string(),
        ];

        // Apply article-specific faults
        self.injector.apply_article_faults(id, &mut headers, &mut body);

        (headers, body)
    }

    pub fn is_compression_enabled(&self) -> bool {
        self.compression_enabled
    }
}

/// Response types
pub struct Response {
    pub status_line: Vec<u8>,
    pub body: Option<Vec<u8>>,
    pub include_terminator: bool,
    pub is_quit: bool,
}

impl Response {
    pub fn single(code: u16, message: &str, injector: &FaultInjector) -> Self {
        let code_str = injector.apply_invalid_code(code);
        let response = format!("{} {}\r\n", code_str, message);
        let status_line = injector.apply_status_faults(&response);

        Self {
            status_line,
            body: None,
            include_terminator: false,
            is_quit: code == 205,
        }
    }

    pub fn multiline(code: u16, message: &str, lines: Vec<String>, injector: &FaultInjector) -> Self {
        let code_str = injector.apply_invalid_code(code);
        let status_response = format!("{} {}\r\n", code_str, message);
        let status_line = injector.apply_status_faults(&status_response);

        let (body, include_terminator) = injector.apply_multiline_faults(&lines);

        Self {
            status_line,
            body: Some(body),
            include_terminator,
            is_quit: false,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut result = self.status_line.clone();
        if let Some(ref body) = self.body {
            result.extend_from_slice(body);
            if self.include_terminator {
                result.extend_from_slice(b".\r\n");
            }
        }
        result
    }
}
