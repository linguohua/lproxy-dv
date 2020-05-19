// This file is generated. Do not edit
// @generated

// https://github.com/Manishearth/rust-clippy/issues/702
#![allow(unknown_lints)]
#![allow(clippy::all)]

#![cfg_attr(rustfmt, rustfmt_skip)]

#![allow(box_pointers)]
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(trivial_casts)]
#![allow(unsafe_code)]
#![allow(unused_imports)]
#![allow(unused_results)]

const METHOD_BANDWIDTH_REPORT_REPORT: ::grpcio::Method<super::dv_import::BandwidthStatistics, super::dv_import::ReportResult> = ::grpcio::Method {
    ty: ::grpcio::MethodType::Unary,
    name: "/BandwidthReport/Report",
    req_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
    resp_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
};

#[derive(Clone)]
pub struct BandwidthReportClient {
    client: ::grpcio::Client,
}

impl BandwidthReportClient {
    pub fn new(channel: ::grpcio::Channel) -> Self {
        BandwidthReportClient {
            client: ::grpcio::Client::new(channel),
        }
    }

    pub fn report_opt(&self, req: &super::dv_import::BandwidthStatistics, opt: ::grpcio::CallOption) -> ::grpcio::Result<super::dv_import::ReportResult> {
        self.client.unary_call(&METHOD_BANDWIDTH_REPORT_REPORT, req, opt)
    }

    pub fn report(&self, req: &super::dv_import::BandwidthStatistics) -> ::grpcio::Result<super::dv_import::ReportResult> {
        self.report_opt(req, ::grpcio::CallOption::default())
    }

    pub fn report_async_opt(&self, req: &super::dv_import::BandwidthStatistics, opt: ::grpcio::CallOption) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv_import::ReportResult>> {
        self.client.unary_call_async(&METHOD_BANDWIDTH_REPORT_REPORT, req, opt)
    }

    pub fn report_async(&self, req: &super::dv_import::BandwidthStatistics) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv_import::ReportResult>> {
        self.report_async_opt(req, ::grpcio::CallOption::default())
    }
    pub fn spawn<F>(&self, f: F) where F: ::futures_01::Future<Item = (), Error = ()> + Send + 'static {
        self.client.spawn(f)
    }
}

pub trait BandwidthReport {
    fn report(&mut self, ctx: ::grpcio::RpcContext, req: super::dv_import::BandwidthStatistics, sink: ::grpcio::UnarySink<super::dv_import::ReportResult>);
}

pub fn create_bandwidth_report<S: BandwidthReport + Send + Clone + 'static>(s: S) -> ::grpcio::Service {
    let mut builder = ::grpcio::ServiceBuilder::new();
    let mut instance = s;
    builder = builder.add_unary_handler(&METHOD_BANDWIDTH_REPORT_REPORT, move |ctx, req, resp| {
        instance.report(ctx, req, resp)
    });
    builder.build()
}

const METHOD_DEVICE_CFG_PULL_PULL_CFG: ::grpcio::Method<super::dv_import::CfgPullRequest, super::dv_import::CfgPullResult> = ::grpcio::Method {
    ty: ::grpcio::MethodType::Unary,
    name: "/DeviceCfgPull/PullCfg",
    req_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
    resp_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
};

#[derive(Clone)]
pub struct DeviceCfgPullClient {
    client: ::grpcio::Client,
}

impl DeviceCfgPullClient {
    pub fn new(channel: ::grpcio::Channel) -> Self {
        DeviceCfgPullClient {
            client: ::grpcio::Client::new(channel),
        }
    }

    pub fn pull_cfg_opt(&self, req: &super::dv_import::CfgPullRequest, opt: ::grpcio::CallOption) -> ::grpcio::Result<super::dv_import::CfgPullResult> {
        self.client.unary_call(&METHOD_DEVICE_CFG_PULL_PULL_CFG, req, opt)
    }

    pub fn pull_cfg(&self, req: &super::dv_import::CfgPullRequest) -> ::grpcio::Result<super::dv_import::CfgPullResult> {
        self.pull_cfg_opt(req, ::grpcio::CallOption::default())
    }

    pub fn pull_cfg_async_opt(&self, req: &super::dv_import::CfgPullRequest, opt: ::grpcio::CallOption) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv_import::CfgPullResult>> {
        self.client.unary_call_async(&METHOD_DEVICE_CFG_PULL_PULL_CFG, req, opt)
    }

    pub fn pull_cfg_async(&self, req: &super::dv_import::CfgPullRequest) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv_import::CfgPullResult>> {
        self.pull_cfg_async_opt(req, ::grpcio::CallOption::default())
    }
    pub fn spawn<F>(&self, f: F) where F: ::futures_01::Future<Item = (), Error = ()> + Send + 'static {
        self.client.spawn(f)
    }
}

pub trait DeviceCfgPull {
    fn pull_cfg(&mut self, ctx: ::grpcio::RpcContext, req: super::dv_import::CfgPullRequest, sink: ::grpcio::UnarySink<super::dv_import::CfgPullResult>);
}

pub fn create_device_cfg_pull<S: DeviceCfgPull + Send + Clone + 'static>(s: S) -> ::grpcio::Service {
    let mut builder = ::grpcio::ServiceBuilder::new();
    let mut instance = s;
    builder = builder.add_unary_handler(&METHOD_DEVICE_CFG_PULL_PULL_CFG, move |ctx, req, resp| {
        instance.pull_cfg(ctx, req, resp)
    });
    builder.build()
}
