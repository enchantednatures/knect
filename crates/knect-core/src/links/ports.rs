//! Recognition of conventional data-service ports.
//!
//! Port classification is intentionally a closed set: only the ports knect can
//! confidently attribute to a backing service protocol are recognized. Any
//! other port is still a candidate, but it never carries protocol confidence on
//! its own.

/// A standard backing-service protocol keyed by its conventional port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PortClass {
    /// Redis or a Redis-compatible cache.
    Redis,
    /// PostgreSQL.
    Postgres,
    /// MySQL or MariaDB.
    Mysql,
    /// Apache Kafka.
    Kafka,
    /// AMQP 0-9-1 (RabbitMQ).
    Amqp,
}

impl PortClass {
    /// Every recognized class, in stable order.
    pub const ALL: [Self; 5] = [
        Self::Redis,
        Self::Postgres,
        Self::Mysql,
        Self::Kafka,
        Self::Amqp,
    ];

    /// Conventional TCP port.
    #[must_use]
    pub const fn port(self) -> u16 {
        match self {
            Self::Redis => 6379,
            Self::Postgres => 5432,
            Self::Mysql => 3306,
            Self::Kafka => 9092,
            Self::Amqp => 5672,
        }
    }

    /// Canonical URI scheme.
    #[must_use]
    pub const fn scheme(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Kafka => "kafka",
            Self::Amqp => "amqp",
        }
    }

    /// Human-readable protocol name used in evidence descriptions.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Redis => "redis",
            Self::Postgres => "postgresql",
            Self::Mysql => "mysql",
            Self::Kafka => "kafka",
            Self::Amqp => "amqp",
        }
    }
}

/// Classifies a TCP port against the closed standard-port set.
#[must_use]
pub fn classify_port(port: u16) -> Option<PortClass> {
    PortClass::ALL
        .into_iter()
        .find(|class| class.port() == port)
}

/// Classifies a URI scheme, including secure variants.
#[must_use]
pub fn class_for_scheme(scheme: &str) -> Option<PortClass> {
    match scheme.to_ascii_lowercase().as_str() {
        "redis" | "rediss" => Some(PortClass::Redis),
        "postgres" | "postgresql" => Some(PortClass::Postgres),
        "mysql" | "mariadb" => Some(PortClass::Mysql),
        "kafka" | "kafkas" => Some(PortClass::Kafka),
        "amqp" | "amqps" => Some(PortClass::Amqp),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_ports_map_to_protocols() {
        let expected = [
            (6379, PortClass::Redis),
            (5432, PortClass::Postgres),
            (3306, PortClass::Mysql),
            (9092, PortClass::Kafka),
            (5672, PortClass::Amqp),
        ];
        for (port, class) in expected {
            assert_eq!(classify_port(port), Some(class));
            assert_eq!(class.port(), port);
        }
        assert_eq!(classify_port(8080), None);
        assert_eq!(classify_port(80), None);
        assert_eq!(classify_port(0), None);
    }

    #[test]
    fn schemes_map_to_protocols_including_secure_variants() {
        assert_eq!(class_for_scheme("redis"), Some(PortClass::Redis));
        assert_eq!(class_for_scheme("REDISS"), Some(PortClass::Redis));
        assert_eq!(class_for_scheme("postgresql"), Some(PortClass::Postgres));
        assert_eq!(class_for_scheme("postgres"), Some(PortClass::Postgres));
        assert_eq!(class_for_scheme("mysql"), Some(PortClass::Mysql));
        assert_eq!(class_for_scheme("amqps"), Some(PortClass::Amqp));
        assert_eq!(class_for_scheme("kafka"), Some(PortClass::Kafka));
        assert_eq!(class_for_scheme("https"), None);
        assert_eq!(class_for_scheme("http"), None);
    }
}
